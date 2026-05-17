use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum DetectedBlock {
    Cloudflare,
    PerimeterX,
    DataDome,
    Akamai,
    Captcha(String), // "recaptcha" or "hcaptcha"
    RateLimit,
    AuthWall,
    Paywall,
    Unknown,
}

pub fn classify_response(
    status: u16,
    body: &str,
    headers: &HashMap<String, String>,
) -> Option<DetectedBlock> {
    let body_lower = body.to_lowercase();

    // Rate limit: 429
    if status == 429 {
        return Some(DetectedBlock::RateLimit);
    }

    // Auth wall: 401
    if status == 401 {
        return Some(DetectedBlock::AuthWall);
    }

    // Only check 403 and 503 for bot detection
    if status != 403 && status != 503 {
        // Check for paywall on 200 pages
        if status == 200 {
            return detect_paywall(&body_lower);
        }
        return None;
    }

    // Cloudflare: cf-ray header + body indicators
    let has_cf = headers.keys().any(|k| k.to_lowercase() == "cf-ray")
        || headers.keys().any(|k| {
            k.to_lowercase() == "server"
                && headers
                    .get(k)
                    .map(|v| v.to_lowercase().contains("cloudflare"))
                    .unwrap_or(false)
        });
    if has_cf
        || body_lower.contains("cloudflare")
        || body_lower.contains("challenge-platform")
        || body_lower.contains("cf-browser-verification")
    {
        return Some(DetectedBlock::Cloudflare);
    }

    // PerimeterX
    if body_lower.contains("_pxcaptcha")
        || body_lower.contains("perimeterx")
        || body_lower.contains("_pxm")
        || body_lower.contains("px-captcha")
    {
        return Some(DetectedBlock::PerimeterX);
    }

    // DataDome
    if body_lower.contains("datadome")
        || headers
            .keys()
            .any(|k| k.to_lowercase().contains("datadome"))
        || headers
            .values()
            .any(|v| v.to_lowercase().contains("datadome"))
    {
        return Some(DetectedBlock::DataDome);
    }

    // Akamai
    if headers
        .keys()
        .any(|k| k.to_lowercase() == "x-akamai-transformed")
        || body_lower.contains("akamai")
        || body_lower.contains("reference #")
    {
        return Some(DetectedBlock::Akamai);
    }

    // reCAPTCHA
    if body_lower.contains("recaptcha") || body_lower.contains("g-recaptcha") {
        return Some(DetectedBlock::Captcha("recaptcha".to_string()));
    }

    // hCaptcha
    if body_lower.contains("hcaptcha") || body_lower.contains("h-captcha") {
        return Some(DetectedBlock::Captcha("hcaptcha".to_string()));
    }

    // Generic 403 without known pattern
    if status == 403 {
        return Some(DetectedBlock::Unknown);
    }

    None
}

fn detect_paywall(body_lower: &str) -> Option<DetectedBlock> {
    let paywall_indicators = [
        ("subscribe to continue", 0.9),
        ("premium content", 0.8),
        ("paywall", 0.95),
        ("subscription required", 0.9),
        ("members only", 0.7),
        ("sign in to read", 0.7),
        ("this article is for subscribers", 0.85),
    ];

    let mut max_confidence = 0.0;
    for (indicator, confidence) in &paywall_indicators {
        if body_lower.contains(indicator) && *confidence > max_confidence {
            max_confidence = *confidence;
        }
    }

    if max_confidence >= 0.8 {
        Some(DetectedBlock::Paywall)
    } else {
        None
    }
}

impl DetectedBlock {
    pub fn to_block_type(&self) -> crate::types::BlockType {
        match self {
            DetectedBlock::Cloudflare => crate::types::BlockType::Cloudflare,
            DetectedBlock::PerimeterX => crate::types::BlockType::PerimeterX,
            DetectedBlock::DataDome => crate::types::BlockType::DataDome,
            DetectedBlock::Akamai => crate::types::BlockType::Akamai,
            DetectedBlock::Captcha(_) => crate::types::BlockType::Captcha,
            DetectedBlock::RateLimit => crate::types::BlockType::RateLimit,
            DetectedBlock::AuthWall => crate::types::BlockType::AuthWall,
            DetectedBlock::Paywall => crate::types::BlockType::Paywall,
            DetectedBlock::Unknown => crate::types::BlockType::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_cloudflare_detection() {
        let headers = make_headers(&[("cf-ray", "abc123"), ("server", "cloudflare")]);
        let result = classify_response(403, "Access denied", &headers);
        assert!(matches!(result, Some(DetectedBlock::Cloudflare)));

        let headers2 = make_headers(&[]);
        let result2 = classify_response(
            403,
            "Please wait while cloudflare verifies your browser",
            &headers2,
        );
        assert!(matches!(result2, Some(DetectedBlock::Cloudflare)));
    }

    #[test]
    fn test_perimeterx_detection() {
        let headers = make_headers(&[]);
        let result = classify_response(403, "window._pxCaptcha = '...'", &headers);
        assert!(matches!(result, Some(DetectedBlock::PerimeterX)));
    }

    #[test]
    fn test_datadome_detection() {
        let headers = make_headers(&[("set-cookie", "datadome=abc")]);
        let result = classify_response(403, "blocked", &headers);
        assert!(matches!(result, Some(DetectedBlock::DataDome)));
    }

    #[test]
    fn test_rate_limit() {
        let headers = make_headers(&[]);
        let result = classify_response(429, "Too many requests", &headers);
        assert!(matches!(result, Some(DetectedBlock::RateLimit)));
    }

    #[test]
    fn test_auth_wall() {
        let headers = make_headers(&[]);
        let result = classify_response(401, "Unauthorized", &headers);
        assert!(matches!(result, Some(DetectedBlock::AuthWall)));
    }

    #[test]
    fn test_captcha_detection() {
        let headers = make_headers(&[]);
        let result = classify_response(403, "Please complete the recaptcha", &headers);
        assert!(matches!(result, Some(DetectedBlock::Captcha(_))));

        let result2 = classify_response(403, "hcaptcha challenge", &headers);
        assert!(matches!(result2, Some(DetectedBlock::Captcha(_))));
    }

    #[test]
    fn test_no_false_positive() {
        let headers = make_headers(&[]);
        let result = classify_response(200, "<html><body>Hello world</body></html>", &headers);
        assert!(result.is_none());
    }

    #[test]
    fn test_paywall_detection() {
        let headers = make_headers(&[]);
        let result = classify_response(
            200,
            "<html><body>Subscribe to continue reading this article</body></html>",
            &headers,
        );
        assert!(matches!(result, Some(DetectedBlock::Paywall)));
    }
}
