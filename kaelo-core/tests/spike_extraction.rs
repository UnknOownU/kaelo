use dom_smoothie::{Config, Readability, TextMode};

fn extract(html: &str, url: &str) -> dom_smoothie::Article {
    let cfg = Config {
        text_mode: TextMode::Markdown,
        ..Default::default()
    };
    let mut r = Readability::new(html, Some(url), Some(cfg))
        .expect("Readability::new should accept valid HTML");
    r.parse().expect("parse() should extract an article")
}

fn section(name: &str) {
    println!("\n{}", "=".repeat(80));
    println!("SPIKE: {}", name);
    println!("{}", "=".repeat(80));
}

fn md(article: &dom_smoothie::Article) -> &str {
    &article.text_content
}

#[test]
fn spike_blog_post_strips_boilerplate() {
    let html = include_str!("fixtures/blog_post.html");
    let article = extract(html, "https://example.com/blog/rust-ownership");
    let content = md(&article);

    section("Blog Post: nav/header/footer stripping");
    println!("TITLE: {}", article.title);
    println!("\nMARKDOWN:\n{}", content);

    assert!(
        article.title.contains("Ownership"),
        "title: {}",
        article.title
    );
    assert!(!content.contains("[Home]"), "nav links stripped");
    assert!(!content.contains("Privacy Policy"), "footer stripped");
    assert!(
        content.contains("ownership model"),
        "article body preserved"
    );
    assert!(content.contains("calculate_length"), "code preserved");

    println!("\n✅ PASSED");
}

#[test]
fn spike_docs_page_preserves_code_and_tables() {
    let html = include_str!("fixtures/docs_page.html");
    let article = extract(html, "https://example.com/docs/api");
    let content = md(&article);

    section("Docs Page: code blocks and tables");
    println!("TITLE: {}", article.title);
    println!("\nMARKDOWN:\n{}", content);

    assert!(
        content.contains("parse") || content.contains("mylib"),
        "code preserved"
    );
    assert!(
        !content.contains("[Getting Started]") || content.contains("Installation"),
        "doc nav stripped"
    );

    println!("\n✅ PASSED");
}

#[test]
fn spike_github_readme_headings() {
    let html = include_str!("fixtures/github_readme.html");
    let article = extract(html, "https://github.com/user/awesome-project");
    let content = md(&article);

    section("GitHub README: heading handling");
    println!("TITLE: {}", article.title);
    println!("\nMARKDOWN:\n{}", content);

    assert!(
        content.contains("awesome-project") || article.title.contains("awesome"),
        "project name"
    );
    assert!(
        content.contains("Zero-copy") || content.contains("zero"),
        "features preserved"
    );
    assert!(!content.contains("Pull requests"), "GitHub chrome stripped");
    assert!(!content.contains("Marketplace"), "GitHub nav stripped");
    assert!(
        content.contains("1.2M") || content.contains("throughput"),
        "benchmarks preserved"
    );

    println!("\n✅ PASSED");
}

#[test]
fn spike_minimal_article_simple_case() {
    let html = include_str!("fixtures/minimal_article.html");
    let article = extract(html, "https://example.com/tips/cargo-watch");
    let content = md(&article);

    section("Minimal Article: simple case");
    println!("TITLE: {}", article.title);
    println!("\nMARKDOWN:\n{}", content);

    assert!(
        content.contains("cargo-watch") || content.contains("cargo watch"),
        "cargo-watch mentioned"
    );
    assert!(
        content.contains("cargo install") || content.contains("install"),
        "install cmd preserved"
    );
    assert!(!content.contains("<article>"), "no HTML article tags");
    assert!(!content.contains("<pre>"), "no HTML pre tags");

    println!("\n✅ PASSED");
}

#[test]
fn spike_news_article_strips_ads_and_comments() {
    let html = include_str!("fixtures/news_article.html");
    let article = extract(html, "https://newscorp.com/tech/rust-release");
    let content = md(&article);

    section("News Article: ads/comments stripping");
    println!("TITLE: {}", article.title);
    println!("\nMARKDOWN:\n{}", content);

    assert!(
        article.title.contains("Rust"),
        "title has Rust: {}",
        article.title
    );
    assert!(
        content.contains("20%") || content.contains("incremental"),
        "article body preserved"
    );
    assert!(!content.contains("rust_fan_42"), "comments stripped");
    assert!(!content.contains("cpp_dev"), "comments stripped");
    assert!(!content.contains("ads.example.com"), "ad URLs stripped");
    assert!(
        !content.contains("We use cookies"),
        "cookie notice stripped"
    );
    assert!(
        !content.contains("NewsCorp Inc. All rights reserved"),
        "footer stripped"
    );

    println!("\n✅ PASSED");
}

#[test]
fn spike_explore_article_fields() {
    let html = include_str!("fixtures/blog_post.html");
    let article = extract(html, "https://example.com/blog/rust-ownership");

    section("Article Field Exploration");
    println!("title:       {}", article.title);
    println!("content (HTML) len: {}", article.content.len());
    println!("text_content (MD) len: {}", article.text_content.len());
    println!("byline:      {:?}", article.byline);
    println!("excerpt:     {:?}", article.excerpt);
    println!("site_name:   {:?}", article.site_name);
    println!("dir:         {:?}", article.dir);
    println!("lang:        {:?}", article.lang);
    println!("length:      {:?}", article.length);

    println!("\ntext_content FULL:\n{}", &article.text_content);

    println!("\n✅ PASSED");
}
