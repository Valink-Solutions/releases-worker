use semver::Version;
use worker::*;
use serde_json::json;
use chrono::{DateTime, FixedOffset};
use reqwest::Client;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct GitHubRelease {
    tag_name: String,
    published_at: String,
    body: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize, Debug)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let router = Router::new();

    router
        .get_async("/:target/:arch/:current_version", get_release)
        .get_async("/download/:target/:arch", get_download)
        .run(req, env)
        .await
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

    let latest_release = match releases.iter().find(|&release| release.tag_name != current_version.to_owned()) {
        Some(release) => release,
        None => return Response::error("No new release found", 404),
    };

    let (file_extension, sig_file_extension) = get_update_extension(&target, &arch);

    if file_extension.is_empty() || sig_file_extension.is_empty() {
        return Response::error("Invalid target", 400);
    }

    let update_asset = match latest_release.assets.iter().find(|asset| asset.name.ends_with(&file_extension)) {
        Some(asset) => asset,
        None => return Response::error("No update asset found", 404),
    };

    let download_url = update_asset.browser_download_url.clone();
    let new_version = latest_release.tag_name.chars().filter(|c| c.is_digit(10) || *c == '.').collect::<String>();

    let pub_date: DateTime<FixedOffset> = match DateTime::parse_from_rfc3339(
        latest_release.published_at.as_str(),
    ) {
        Ok(pub_date) => pub_date,
        Err(_) => return Response::error("Failed to parse published date", 500),
    };

    let notes = latest_release.body.clone();
    let signature_asset = match latest_release.assets.iter().find(|asset| asset.name.ends_with(&sig_file_extension)) {
        Some(asset) => asset,
        None => return Response::error("No signature asset found", 404),
    };
    let signature_url = signature_asset.browser_download_url.clone();

    let signature_resp = match client.get(signature_url).send().await {
        Ok(resp) => resp,
        Err(_) => return Response::error("Failed to fetch signature", 500),
    };

    let signature = match signature_resp.text().await {
        Ok(signature) => signature,
        Err(_) => return Response::error("Failed to parse signature", 500),
    };

    let response_body = json!({
        "version": new_version,
        "pub_date": pub_date.to_rfc3339(),
        "url": download_url,
        "signature": signature,
        "notes": clean_markdown(&notes)
    });

    console_log!("{} {} got response: {}", target, arch, response_body);

    let mut response = Response::from_json(&response_body)?;

    response.headers_mut().set("Content-Type", "application/json").unwrap();

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

    let file_extension = get_download_extension(&target, &arch);

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

    Response::redirect(download_url)
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