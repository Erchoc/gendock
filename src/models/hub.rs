use super::ModelError;
use anyhow::{Result, ensure};
use reqwest::{Url, blocking::Client};
use serde_json::json;
use std::{
    path::{Component, Path},
    time::Duration,
};

pub struct Hub {
    pub client: Client,
    pub endpoint: Url,
}
impl Hub {
    pub fn new(endpoint: &str, proxy: Option<&str>) -> Result<Self> {
        let endpoint = match endpoint {
            "hf" => "https://huggingface.co",
            "hf-mirror" => "https://hf-mirror.com",
            other => other,
        };
        let url = Url::parse(endpoint).map_err(|_| config_error())?;
        let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if !(url.scheme() == "https" || (url.scheme() == "http" && local))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(config_error().into());
        }
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .user_agent(concat!("gendock/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    return attempt.error("too many redirects");
                }
                if attempt.url().scheme() != "https"
                    && !matches!(
                        attempt.url().host_str(),
                        Some("127.0.0.1" | "localhost" | "[::1]")
                    )
                {
                    return attempt.error("insecure redirect");
                }
                attempt.follow()
            }));
        if let Some(proxy) = proxy {
            let proxy_url = Url::parse(proxy).map_err(|_| config_error())?;
            if !matches!(proxy_url.scheme(), "http" | "https" | "socks5" | "socks5h")
                || proxy_url.host_str().is_none()
            {
                return Err(config_error().into());
            }
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| config_error())?);
        }
        let client = builder.build().map_err(|_| config_error())?;
        Ok(Self {
            client,
            endpoint: url,
        })
    }

    pub fn file_url(&self, repo: &str, revision: &str, path: &str) -> Result<Url> {
        validate_path(path)?;
        ensure!(
            repo.split('/').count() == 2
                && repo
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != ".."),
            "invalid repository"
        );
        ensure!(
            revision.len() == 40 && revision.bytes().all(|b| b.is_ascii_hexdigit()),
            "revision must be a commit hash"
        );
        let mut url = self.endpoint.clone();
        url.path_segments_mut()
            .map_err(|_| config_error())?
            .clear()
            .extend(repo.split('/'))
            .push("resolve")
            .push(revision)
            .extend(path.split('/'));
        Ok(url)
    }

    pub fn download_url(&self, model: &super::Model, file: &super::File) -> Result<Url> {
        if let Some(source) = &file.url {
            let url = validate_url(source)?;
            if url.host_str() == Some("huggingface.co") {
                let mut mirror = self.endpoint.join(url.path()).map_err(|_| config_error())?;
                mirror.set_query(url.query());
                return Ok(mirror);
            }
            return Ok(url);
        }
        self.file_url(&model.repository, &model.revision, &file.path)
    }

    pub fn request(&self, url: Url) -> reqwest::blocking::RequestBuilder {
        let official = url.scheme() == "https"
            && url.host_str() == Some("huggingface.co")
            && url.port_or_known_default() == Some(443);
        let mut request = self.client.get(url);
        // Never send the user's official HF token to a mirror, R2 or custom endpoint.
        if official
            && self.endpoint.host_str() == Some("huggingface.co")
            && let Ok(token) = std::env::var("HF_TOKEN")
        {
            request = request.bearer_auth(token);
        }
        request
    }
}

pub fn validate_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| config_error())?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if url.host_str().is_none()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(config_error().into());
    }
    Ok(url)
}

fn config_error() -> ModelError {
    ModelError {
        code: "invalid_network_config",
        message: "Endpoint or proxy configuration is invalid.",
        retryable: false,
        next_step: "Use --endpoint hf or --endpoint hf-mirror. Use --proxy with a valid HTTP or SOCKS proxy URL. Credentials are not allowed in endpoint URLs.",
        details: json!({}),
    }
}

pub fn network_error(status: Option<u16>) -> ModelError {
    let (code, message, retryable, next) = match status {
        Some(401 | 403) => (
            "model_access_denied",
            "Model access was denied.",
            false,
            "Check model access and license requirements on Hugging Face. HF_TOKEN is used only with the official endpoint.",
        ),
        Some(404) => (
            "remote_file_not_found",
            "A pinned model file is unavailable at this endpoint.",
            false,
            "Try the official --endpoint hf or check whether the mirror contains this exact revision.",
        ),
        Some(429 | 500..=599) => (
            "download_unavailable",
            "The download service is temporarily unavailable.",
            true,
            "Retry with bounded backoff; partial downloads are retained.",
        ),
        _ => (
            "download_failed",
            "The download request failed.",
            true,
            "Check network connectivity and endpoint/proxy settings, then retry to resume.",
        ),
    };
    ModelError {
        code,
        message,
        retryable,
        next_step: next,
        details: json!({"http_status":status}),
    }
}

pub fn validate_path(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && !value.contains('\\')
            && !value.contains('\0')
            && !value.contains(':')
            && value
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != "..")
            && Path::new(value)
                .components()
                .all(|p| matches!(p, Component::Normal(_))),
        "unsafe model file path"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mirror_rewrites_only_official_hf_sources() {
        let model = crate::models::catalog().remove(0);
        let hub = Hub::new("hf-mirror", None).unwrap();
        let mut file = model.files[0].clone();
        let mirrored = hub.download_url(&model, &file).unwrap();
        assert_eq!(mirrored.host_str(), Some("hf-mirror.com"));
        assert!(mirrored.path().contains(&model.revision));
        file.url = Some("https://models.example.com/r2/weights.bin".into());
        assert_eq!(
            hub.download_url(&model, &file).unwrap().as_str(),
            file.url.as_deref().unwrap()
        );
    }

    #[test]
    fn rejects_path_escape_and_endpoint_secrets() {
        for path in ["../secret", "/absolute", "a/../b", "a\\b", "a//b"] {
            assert!(validate_path(path).is_err());
        }
        assert!(Hub::new("https://user:secret@example.com", None).is_err());
        assert!(Hub::new("http://example.com", None).is_err());
        assert!(Hub::new("hf-mirror", None).is_ok());
    }
}
