use semver::Version;
use worker::*;
use serde_json::json;
use chrono::{DateTime, FixedOffset};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default)]
struct GitHubRelease {
    tag_name: String,
    published_at: String,
    body: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Serialize, Deserialize, Debug, Default)] 
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    download_count: i64,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct TotalDownloads {
    total_downloads: i64,
    updated_at: String,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct RecentRelease {
    version: String,
    pub_date: String,
    url: String,
    signature: String,
    notes: String,
    releases: Vec<GitHubRelease>,
    updated_at: String,
}


#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let router = Router::new();

    router
        .get_async("/:target/:arch/:current_version", get_release)
        .get_async("/download/:target/:arch", get_download)
        .get_async("/total_downloads", get_total_downloads)
        .run(req, env)
        .await
}

async fn get_total_downloads(_req: worker::Request, ctx: RouteContext<()>) -> Result<Response> {
    let kv = ctx.kv("KV_CHUNKVAULT_DOWNLOADS");

    let old_downloads = if let Ok(kv) = &kv {
        kv.get("recent_download_count").json::<TotalDownloads>().await.ok().unwrap()
    } else {
        None
    };

    let updated_at = match &old_downloads {
        Some(downloads) => DateTime::parse_from_rfc3339(&downloads.updated_at).unwrap_or_else(|_| DateTime::<FixedOffset>::from(chrono::Utc::now())),
        None => DateTime::<FixedOffset>::from(chrono::Utc::now()),
    };

    if updated_at.timestamp() + 300 > chrono::Utc::now().timestamp() {
        if let Some(old_downloads) = old_downloads {
            return Ok(Response::from_json(&old_downloads)?);
        }
    }

    let client = Client::new();
    let url = "https://api.github.com/repos/Valink-Solutions/teller/releases";
    let resp = client.get(url)
        .header("User-Agent", "chunkvault-updater")
        .send()
        .await
        .map_err(|_| "Failed to fetch releases")?;

    let releases: Vec<GitHubRelease> = resp.json().await.map_err(|_| "Failed to parse releases")?;

    let total_downloads: i64 = releases.iter()
        .flat_map(|release| &release.assets)
        .map(|asset| asset.download_count)
        .sum();

    let new_downloads = TotalDownloads {
        total_downloads: total_downloads,
        updated_at: releases[0].published_at.to_string(),
    };

    if let Ok(kv) = kv {
        if let Ok(kv_action) = kv.put("recent_download_count", &new_downloads) {
            let _ = kv_action.execute().await;
        }
    };

    Ok(Response::from_json(&new_downloads)?)
}


async fn get_release(_req: worker::Request, ctx: RouteContext<()>) -> Result<Response> {
    let target = match ctx.param("target") {
        Some(target) => target,
        None => return Response::error("Missing target", 400),
    };
    let arch = match ctx.param("arch") {
        Some(arch) => arch,
        None => return Response::error("Missing arch", 400),
    };
    let current_version = match ctx.param("current_version") {
        Some(current_version) => current_version,
        None => return Response::error("Missing current_version", 400),
    };

    let kv = ctx.kv("KV_CHUNKVAULT_DOWNLOADS");

    let mut old_release = if let Ok(kv) = &kv {
        let old_release: RecentRelease = kv.get("recent_release").json::<RecentRelease>().await.unwrap().unwrap();
        old_release
    } else {
        RecentRelease::default()
    };

    let updated_at = match DateTime::parse_from_rfc3339(&old_release.updated_at.as_str()) {
        Ok(date) => date,
        Err(_) => DateTime::<FixedOffset>::from(chrono::Utc::now()),
    };
    
    if updated_at.timestamp() + 300 > chrono::Utc::now().timestamp() {
        return match parse_releases(old_release, target.to_string(), arch.to_string(), current_version.to_string()).await {
            Ok(release) => {
                let mut response = Response::from_json(&json!(
                    {
                        "version": release.version,
                        "pub_date": release.pub_date,
                        "url": release.url,
                        "signature": release.signature,
                        "notes": release.notes,
                    }
                ))?;

                response.headers_mut().set("Content-Type", "application/json").unwrap();

                Ok(response)
            },
            Err(err) => Response::error(err, 500),
        };
    } else {
        let client = Client::new();
        let url = "https://api.github.com/repos/Valink-Solutions/teller/releases";
        let resp = match client.get(url)
            .header("User-Agent", "chunkvault-updater")
            .send()
            .await {
            Ok(resp) => resp,
            Err(_) => return Response::error("Failed to fetch releases", 500),
        };

        let releases: Vec<GitHubRelease> = match resp.json().await {
            Ok(releases) => releases,
            Err(_) => return Response::error("Failed to parse releases", 500),
        };

        old_release.releases = releases;


        return match parse_releases(old_release, target.to_string(), arch.to_string(), current_version.to_string()).await {
            Ok(release) => {
                if let Ok(kv) = kv {
                    if let Ok(kv_action) = kv.put("recent_download_count", &release) {
                        let _ = kv_action.execute().await;
                    }
                };

                let mut response = Response::from_json(&json!(
                    {
                        "version": release.version,
                        "pub_date": release.pub_date,
                        "url": release.url,
                        "signature": release.signature,
                        "notes": release.notes,
                    }
                ))?;

                response.headers_mut().set("Content-Type", "application/json").unwrap();

                Ok(response)
            },
            Err(err) => Response::error(err, 500),
        };
    }
}

async fn parse_releases(releases: RecentRelease, target: String, arch: String, current_version: String) -> std::result::Result<RecentRelease, String> {
    let latest_release = match releases.releases.iter().find(|&release| release.tag_name != current_version.to_owned()) {
        Some(release) => release,
        None => return Err("No new release found".to_string()),
    };

    let (file_extension, sig_file_extension) = get_update_extension(&target, &arch);

    if file_extension.is_empty() || sig_file_extension.is_empty() {
        return Err("Invalid target".to_string());
    }

    let updated_at = match DateTime::parse_from_rfc3339(latest_release.published_at.as_str()) {
        Ok(date) => date,
        Err(_) => DateTime::<FixedOffset>::from(chrono::Utc::now()),
    };

    let update_asset = match latest_release.assets.iter().find(|asset| asset.name.ends_with(&file_extension)) {
        Some(asset) => asset,
        None => return Err("No update asset found".to_string()),
    };

    let download_url = update_asset.browser_download_url.clone();
    let new_version = latest_release.tag_name.chars().filter(|c| c.is_digit(10) || *c == '.').collect::<String>();

    let pub_date: DateTime<FixedOffset> = match DateTime::parse_from_rfc3339(
        latest_release.published_at.as_str(),
    ) {
        Ok(pub_date) => pub_date,
        Err(_) => return Err("Failed to parse published date".to_string()),
    };

    let notes = latest_release.body.clone();
    let signature_asset = match latest_release.assets.iter().find(|asset| asset.name.ends_with(&sig_file_extension)) {
        Some(asset) => asset,
        None => return Err("No signature asset found".to_string()),
    };
    let signature_url = signature_asset.browser_download_url.clone();
    let client = Client::new();

    let signature_resp = match client.get(signature_url).send().await {
        Ok(resp) => resp,
        Err(_) => return Err("Failed to fetch signature".to_string()),
    };

    let signature = match signature_resp.text().await {
        Ok(signature) => signature,
        Err(_) => return Err("Failed to parse signature".to_string()),
    };

    let response = RecentRelease {
        version: new_version,
        pub_date: pub_date.to_rfc3339(),
        url: download_url,
        signature: signature,
        notes: clean_markdown(&notes),
        releases: releases.releases,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(response)
}

async fn get_download(_req: worker::Request, ctx: RouteContext<()>) -> Result<Response> {
    let target = match ctx.param("target") {
        Some(target) => target,
        None => return Response::error("Missing target", 400),
    };
    let arch = match ctx.param("arch") {
        Some(arch) => arch,
        None => return Response::error("Missing arch", 400),
    };

    let kv = ctx.kv("KV_CHUNKVAULT_DOWNLOADS");

    let old_release = if let Ok(kv) = &kv {
        let old_release: RecentRelease = kv.get("recent_releases").json::<RecentRelease>().await.unwrap().unwrap();
        old_release
    } else {
        RecentRelease::default()
    };

    let old_downloads = if let Ok(kv) = &kv {
        let old_downloads: TotalDownloads = kv.get("recent_total_download").json::<TotalDownloads>().await.unwrap().unwrap();
        old_downloads
    } else {
        TotalDownloads::default()
    };
    
    // If the value is older than 5 minutes, return it else fetch a new value
    let updated_at = match DateTime::parse_from_rfc3339(old_downloads.updated_at.as_str()) {
        Ok(date) => date,
        Err(_) => DateTime::<FixedOffset>::from(chrono::Utc::now()),
    };

    let file_extension = get_download_extension(&target, &arch);

    if updated_at.timestamp() + 300 > chrono::Utc::now().timestamp() {
        let latest_release = match old_release.releases.iter().max_by(|a, b| {
            let version_a = Version::parse(a.tag_name.trim_start_matches('v')).unwrap_or_else(|_| Version::new(0, 0, 0));
            let version_b = Version::parse(b.tag_name.trim_start_matches('v')).unwrap_or_else(|_| Version::new(0, 0, 0));
            version_a.cmp(&version_b)
        }) {
            Some(release) => release,
            None => return Response::error("No new release found", 404),
        };
    
        let download_url_str = match latest_release.assets.iter().find(|asset| {
            asset.name.ends_with(&file_extension)
        }) {
            Some(asset) => &asset.browser_download_url,
            None => return Response::error("No asset found for target", 404),
        };
    
        let download_url = match Url::parse(download_url_str) {
            Ok(url) => url,
            Err(_) => return Response::error("Invalid URL", 400),
        };

        let new_downloads = TotalDownloads {
            total_downloads: old_downloads.total_downloads + 1,
            updated_at: DateTime::parse_from_rfc3339(
                old_release.releases[0].published_at.as_str(),
            ).map_err(|_| "Failed to parse published date")?.to_rfc3339(),
        };

        if let Ok(kv) = &kv {
            if let Ok(kv_action) = kv.put("recent_download_count", &new_downloads) {
                let _ = kv_action.execute().await;
            }
        };

        return Response::redirect(download_url)
    } else {

        let client = Client::new();
        let url = "https://api.github.com/repos/Valink-Solutions/teller/releases";
        let resp = match client.get(url)
            .header("User-Agent", "chunkvault-updater")
            .send()
            .await {
            Ok(resp) => resp,
            Err(_) => return Response::error("Failed to fetch releases", 500),
        };
    
        let releases: Vec<GitHubRelease> = match resp.json().await {
            Ok(releases) => releases,
            Err(_) => return Response::error("Failed to parse releases", 500),
        };
    
        let latest_release = match releases.iter().max_by(|a, b| {
            let version_a = Version::parse(a.tag_name.trim_start_matches('v')).unwrap_or_else(|_| Version::new(0, 0, 0));
            let version_b = Version::parse(b.tag_name.trim_start_matches('v')).unwrap_or_else(|_| Version::new(0, 0, 0));
            version_a.cmp(&version_b)
        }) {
            Some(release) => release,
            None => return Response::error("No new release found", 404),
        };
    
        let download_url_str = match latest_release.assets.iter().find(|asset| {
            asset.name.ends_with(&file_extension)
        }) {
            Some(asset) => &asset.browser_download_url,
            None => return Response::error("No asset found for target", 404),
        };
    
        let download_url = match Url::parse(download_url_str) {
            Ok(url) => url,
            Err(_) => return Response::error("Invalid URL", 400),
        };

        let new_release = parse_releases(old_release, target.to_string(), arch.to_string(), "0.0.0".to_string()).await?;
        
        if let Ok(kv) = &kv {
            if let Ok(kv_action) = kv.put("recent_releases", &new_release) {
                let _ = kv_action.execute().await;
            }
        };

        let total_downloads: i64 = new_release.releases.iter()
            .flat_map(|release| &release.assets)
            .map(|asset| asset.download_count)
            .sum();

        let new_downloads = TotalDownloads {
            total_downloads: total_downloads + 1,
            updated_at: DateTime::parse_from_rfc3339(
                new_release.releases[0].published_at.as_str(),
            ).map_err(|_| "Failed to parse published date")?.to_rfc3339(),
        };
        
        if let Ok(kv) = &kv {
            if let Ok(kv_action) = kv.put("recent_total_downloads", &new_downloads) {
                let _ = kv_action.execute().await;
            }
        };

        if let Ok(kv) = &kv {
            if let Ok(kv_action) = kv.put("recent_releases", &new_release) {
                let _ = kv_action.execute().await;
            }
        };

        Response::redirect(download_url)
    }
}

fn get_download_extension(target: &str, _arch: &str) -> String {
    match target.to_lowercase().as_str() {
        "mac" => ".dmg".to_string(),
        "macos" => ".dmg".to_string(),
        "darwin" => ".dmg".to_string(),
        "linux" => ".AppImage".to_string(),
        "windows" => "-setup.exe".to_string(),
        _ => "".to_string(),
    }
}

fn get_update_extension(target: &str, _arch: &str) -> (String, String) {
    match target.to_lowercase().as_str() {
        "mac" => (".app.tar.gz".to_string(), ".app.tar.gz.sig".to_string()),
        "macos" => (".app.tar.gz".to_string(), ".app.tar.gz.sig".to_string()),
        "darwin" => (".app.tar.gz".to_string(), ".app.tar.gz.sig".to_string()),
        "linux" => (".AppImage.tar.gz".to_string(), ".AppImage.tar.gz.sig".to_string()),
        "windows" => (".nsis.zip".to_string(), ".nsis.zip.sig".to_string()),
        _ => ("".to_string(), "".to_string()),
    }
}

fn clean_markdown(markdown: &str) -> String {
    let header_re = regex::Regex::new(r"(?m)^#+").unwrap();
    let bold_re = regex::Regex::new(r"\*\*(.*?)\*\*").unwrap();
    let italic_re = regex::Regex::new(r"_(.*?)_").unwrap();
    let link_re = regex::Regex::new(r"\[(.*?)\]\(.*?\)").unwrap();
    let specific_text_re = regex::Regex::new(r"\*\*_See the assets to download and install this version\._\*\*").unwrap();
    let notes_re = regex::Regex::new(r"### Notes").unwrap();

    let no_specific_text = specific_text_re.replace_all(&markdown, "");
    let no_notes = notes_re.replace_all(&no_specific_text, "");
    let no_headers = header_re.replace_all(&no_notes, "");
    let no_bold = bold_re.replace_all(&no_headers, "$1");
    let no_italic = italic_re.replace_all(&no_bold, "$1");
    let cleaned_text = link_re.replace_all(&no_italic, "$1");

    cleaned_text.to_string()
}