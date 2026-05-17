use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::types::{FetchError, FetchRequest, FetchResponse, Strategy};

use crate::fetch::backend::FetchBackend;

pub struct HttpSimple {
    client: reqwest::Client,
}

impl HttpSimple {
    pub fn new() -> Result<Self, FetchError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| FetchError::NetworkError(e.to_string()))?;
        Ok(Self { client })
    }
}

impl FetchBackend for HttpSimple {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let start = Instant::now();

        let mut req = self.client.get(&request.url);

        for (key, value) in &request.headers {
            req = req.header(key.as_str(), value.as_str());
        }

        req = req.timeout(request.timeout);

        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                FetchError::Timeout
            } else {
                FetchError::NetworkError(e.to_string())
            }
        })?;

        let status = resp.status().as_u16();

        // Handle 304 Not Modified — return empty body with original headers.
        if status == 304 {
            let headers: HashMap<String, String> = resp
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    let val = v.to_str().ok()?;
                    Some((k.to_string(), val.to_string()))
                })
                .collect();
            let latency = start.elapsed();
            return Ok(FetchResponse {
                status,
                headers,
                body: Vec::new(),
                content_type: String::new(),
                latency,
            });
        }
        let headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                let val = v.to_str().ok()?;
                Some((k.to_string(), val.to_string()))
            })
            .collect();

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = resp
            .bytes()
            .await
            .map_err(|e| FetchError::NetworkError(e.to_string()))?;

        let latency = start.elapsed();

        if status >= 400 {
            return Err(FetchError::HttpError(status));
        }

        Ok(FetchResponse {
            status,
            headers,
            body: body.to_vec(),
            content_type,
            latency,
        })
    }

    fn name(&self) -> Strategy {
        Strategy::HttpSimple
    }

    fn supports_probe(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_simple_new() {
        let client = HttpSimple::new();
        assert!(client.is_ok());
    }

    #[test]
    fn test_http_simple_name() {
        let client = HttpSimple::new().expect("client creation");
        assert_eq!(client.name(), Strategy::HttpSimple);
    }

    #[test]
    fn test_http_simple_supports_probe() {
        let client = HttpSimple::new().expect("client creation");
        assert!(client.supports_probe());
    }
}
