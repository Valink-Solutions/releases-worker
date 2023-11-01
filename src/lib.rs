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
        .run(req, env)
        .await
}

async fn get_release(_req: worker::Request, ctx: RouteContext<()>) -> Result<Response> {
    let target = ctx.param("target").unwrap();
    let arch = ctx.param("arch").unwrap();
    let current_version = ctx.param("current_version").unwrap();

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

    let (file_extension, sig_file_extension) = get_file_extension(&target, &arch);

    if file_extension.is_empty() || sig_file_extension.is_empty() {
        return Response::error("Invalid target", 400);
    }

    let update_asset = match latest_release.assets.iter().find(|asset| asset.name.ends_with(&file_extension)) {
        Some(asset) => asset,
        None => return Response::error("No update asset found", 404),
    };

    let download_url = update_asset.browser_download_url.clone();
    let new_version = latest_release.tag_name.clone();

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
    }).to_string();

    Ok(Response::from_json(&response_body)?)
}

fn get_file_extension(target: &str, _arch: &str) -> (String, String) {
    match target {
        "darwin" => (".app.tar.gz".to_string(), ".app.tar.gz.sig".to_string()),
        "linux" => (".AppImage.tar.gz".to_string(), ".AppImage.tar.gz.sig".to_string()),
        "windows" => (".nsis.zip".to_string(), ".nsis.zip.sig".to_string()),
        _ => ("".to_string(), "".to_string()),
    }
}

fn clean_markdown(markdown: &str) -> String {
    let header_re = regex::Regex::new(r"(?m)^#+.*\n?").unwrap();
    let bold_re = regex::Regex::new(r"\*\*.*?\*\*").unwrap();
    let italic_re = regex::Regex::new(r"_.*?_").unwrap();
    let link_re = regex::Regex::new(r"\[.*?\]\(.*?\)").unwrap();
    let specific_text_re = regex::Regex::new(r"\*\*_See the assets to download and install this version\._\*\*").unwrap();

    let no_headers = header_re.replace_all(markdown, "");
    let no_bold = bold_re.replace_all(&no_headers, "");
    let no_italic = italic_re.replace_all(&no_bold, "");
    let no_links = link_re.replace_all(&no_italic, "");
    let cleaned_text = specific_text_re.replace_all(&no_links, "");

    cleaned_text.to_string()
}