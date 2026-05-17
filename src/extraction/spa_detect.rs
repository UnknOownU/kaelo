//! SPA / JS framework fingerprint detection in raw HTML.
//!
//! Uses only string matching (no regex, no external crates) to identify
//! common server-side-rendered SPA shells that contain no meaningful content.

/// Detected SPA framework.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaFramework {
    Blazor,
    React,
    NextJs,
    Vue,
    Nuxt,
    Angular,
    Astro,
    SvelteKit,
    GenericSpa,
}

/// Detect whether raw HTML is a SPA shell and identify the framework.
///
/// Checks are ordered so that more specific frameworks (NextJs, Nuxt) are
/// detected before their generic counterparts (React, Vue).
pub fn detect_spa(html: &str) -> Option<SpaFramework> {
    // --- Blazor ---
    if html.contains("_framework/blazor")
        || html.contains("<!--Blazor:")
        || html.contains(r#"id="components-reconnect-modal""#)
    {
        return Some(SpaFramework::Blazor);
    }

    // --- NextJs (must be before React) ---
    if html.contains("__NEXT_DATA__") || html.contains("_next/static") {
        return Some(SpaFramework::NextJs);
    }

    // --- Nuxt (must be before Vue) ---
    if html.contains("__NUXT__") || html.contains("data-nuxt") {
        return Some(SpaFramework::Nuxt);
    }

    // --- Angular ---
    if html.contains("<app-root>") || html.contains("ng-version") || html.contains("<ng-app>") {
        return Some(SpaFramework::Angular);
    }

    // --- Astro ---
    if html.contains("data-astro-transition") || html.contains("data-astro-cid") {
        return Some(SpaFramework::Astro);
    }

    // --- SvelteKit ---
    if html.contains("$app/") || html.contains("data-sveltekit") {
        return Some(SpaFramework::SvelteKit);
    }

    // --- React (generic) ---
    if html.contains(r#"<div id="root">"#) && is_spa_body(html) {
        return Some(SpaFramework::React);
    }

    // --- Vue (generic) ---
    if html.contains(r#"<div id="app">"#) && is_spa_body(html) {
        return Some(SpaFramework::Vue);
    }

    // --- GenericSpa: body contains ONLY script tags and whitespace ---
    if is_spa_body(html) && has_spa_mount(html) {
        return Some(SpaFramework::GenericSpa);
    }

    None
}

fn is_spa_body(html: &str) -> bool {
    let body_start = match html.find("<body") {
        Some(i) => i,
        None => return false,
    };

    // Find the closing > of <body ...>
    let after_body_open = match html[body_start..].find('>') {
        Some(i) => body_start + i + 1,
        None => return false,
    };

    let body_end = match html.rfind("</body>") {
        Some(i) => i,
        None => html.len(),
    };

    let body_content = &html[after_body_open..body_end];

    // Check each non-whitespace line is a script tag or HTML comment
    for line in body_content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("<script")
            || trimmed.starts_with("</script")
            || trimmed.starts_with("<!--")
        {
            continue;
        }
        // Allow div#root / div#app mounts (they are the mount points)
        if trimmed.starts_with("<div") {
            continue;
        }
        return false;
    }
    true
}

fn has_spa_mount(html: &str) -> bool {
    html.contains(r#"<div id="root">"#)
        || html.contains(r#"<div id="app">"#)
        || html.contains(r#"<div id="#)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_blazor() {
        let html = r#"<!DOCTYPE html>
<html><head></head><body>
<div id="components-reconnect-modal"></div>
<script src="_framework/blazor.server.js"></script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::Blazor));
    }

    #[test]
    fn detects_blazor_via_comment() {
        let html = r#"<html><head></head><body><!--Blazor:{"type":"server"}-->
<script src="_framework/blazor.server.js"></script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::Blazor));
    }

    #[test]
    fn detects_nextjs() {
        let html = r#"<html><head><script id="__NEXT_DATA__" type="application/json">{"props":{}}</script></head><body>
<div id="__next"></div>
<script src="/_next/static/chunks/main.js"></script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::NextJs));
    }

    #[test]
    fn detects_react() {
        let html = r#"<html><head></head><body>
<div id="root"></div>
<script src="/static/js/main.js"></script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::React));
    }

    #[test]
    fn detects_vue() {
        let html = r#"<html><head></head><body>
<div id="app"></div>
<script src="/js/app.js"></script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::Vue));
    }

    #[test]
    fn detects_nuxt() {
        let html = r#"<html><head><script>window.__NUXT__={};</script></head><body>
<div id="__nuxt"></div>
<script src="/_nuxt/app.js"></script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::Nuxt));
    }

    #[test]
    fn detects_angular() {
        let html = r#"<html><head></head><body>
<app-root></app-root>
<script src="/main.js"></script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::Angular));
    }

    #[test]
    fn detects_astro() {
        let html = r#"<html><head></head><body>
<div data-astro-cid="abc123">content</div>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::Astro));
    }

    #[test]
    fn detects_sveltekit() {
        let html = r#"<html><head></head><body>
<div id="app" data-sveltekit-preload-data="hover"></div>
<script>import { page } from '$app/stores';</script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::SvelteKit));
    }

    #[test]
    fn detects_generic_spa() {
        let html = r#"<html><head></head><body>
<div id="root"></div>
<script src="/bundle.js"></script>
</body></html>"#;
        assert_eq!(detect_spa(html), Some(SpaFramework::React));
    }

    #[test]
    fn non_spa_html_returns_none() {
        let html = r#"<!DOCTYPE html>
<html><head><title>Blog</title></head><body>
<h1>Welcome to my blog</h1>
<p>This is a real article with content.</p>
<article>Some more text here.</article>
</body></html>"#;
        assert_eq!(detect_spa(html), None);
    }

    #[test]
    fn empty_input_returns_none() {
        assert_eq!(detect_spa(""), None);
    }
}
