use kaelo_core::types::{FetchError, FetchRequest, FetchResponse, Strategy};

use crate::backend::FetchBackend;

pub struct HeadlessBrowser;

impl Default for HeadlessBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessBrowser {
    pub fn new() -> Self {
        Self
    }
}

impl FetchBackend for HeadlessBrowser {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchResponse, FetchError> {
        Err(FetchError::BrowserError(
            "headless browser not yet implemented".into(),
        ))
    }

    fn name(&self) -> Strategy {
        Strategy::Headless
    }

    fn supports_probe(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_headless_new() {
        let _browser = HeadlessBrowser::new();
    }

    #[test]
    fn test_headless_name() {
        let browser = HeadlessBrowser::new();
        assert_eq!(browser.name(), Strategy::Headless);
    }
}
