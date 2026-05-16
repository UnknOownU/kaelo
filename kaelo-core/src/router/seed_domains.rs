pub struct SeedDomain {
    pub domain: &'static str,
    pub strategy: &'static str,
    pub notes: &'static str,
}

pub const SEED_DOMAINS: &[SeedDomain] = &[
    SeedDomain {
        domain: "medium.com",
        strategy: "TlsChrome",
        notes: "Cloudflare challenge on articles",
    },
    SeedDomain {
        domain: "discord.com",
        strategy: "TlsChrome",
        notes: "Cloudflare-protected",
    },
    SeedDomain {
        domain: "reddit.com",
        strategy: "PublicApi",
        notes: "JSON API avoids Cloudflare",
    },
    SeedDomain {
        domain: "old.reddit.com",
        strategy: "PublicApi",
        notes: "JSON API avoids Cloudflare",
    },
    SeedDomain {
        domain: "notion.so",
        strategy: "TlsChrome",
        notes: "Cloudflare, JS-rendered",
    },
    SeedDomain {
        domain: "vercel.com",
        strategy: "TlsChrome",
        notes: "Cloudflare",
    },
    SeedDomain {
        domain: "linear.app",
        strategy: "TlsChrome",
        notes: "Cloudflare, SPA",
    },
    SeedDomain {
        domain: "figma.com",
        strategy: "TlsChrome",
        notes: "Cloudflare, JS-heavy",
    },
    SeedDomain {
        domain: "shopify.com",
        strategy: "TlsChrome",
        notes: "Cloudflare storefronts",
    },
    SeedDomain {
        domain: "zendesk.com",
        strategy: "TlsChrome",
        notes: "Cloudflare help centers",
    },
    SeedDomain {
        domain: "wix.com",
        strategy: "TlsChrome",
        notes: "Cloudflare, JS-rendered",
    },
    SeedDomain {
        domain: "stripe.com",
        strategy: "TlsChrome",
        notes: "Cloudflare docs",
    },
    SeedDomain {
        domain: "cloudflare.com",
        strategy: "TlsChrome",
        notes: "Self-protected",
    },
    SeedDomain {
        domain: "docs.rs",
        strategy: "HttpSimple",
        notes: "Rust crate docs, no bot protection",
    },
    SeedDomain {
        domain: "developer.mozilla.org",
        strategy: "HttpSimple",
        notes: "MDN, open access",
    },
    SeedDomain {
        domain: "docs.python.org",
        strategy: "HttpSimple",
        notes: "Python docs",
    },
    SeedDomain {
        domain: "doc.rust-lang.org",
        strategy: "HttpSimple",
        notes: "Rust docs",
    },
    SeedDomain {
        domain: "go.dev",
        strategy: "HttpSimple",
        notes: "Go docs",
    },
    SeedDomain {
        domain: "pkg.go.dev",
        strategy: "HttpSimple",
        notes: "Go package docs",
    },
    SeedDomain {
        domain: "docs.oracle.com",
        strategy: "HttpSimple",
        notes: "Oracle docs",
    },
    SeedDomain {
        domain: "learn.microsoft.com",
        strategy: "HttpSimple",
        notes: "Microsoft Learn",
    },
    SeedDomain {
        domain: "kubernetes.io",
        strategy: "HttpSimple",
        notes: "K8s docs",
    },
    SeedDomain {
        domain: "docs.ansible.com",
        strategy: "HttpSimple",
        notes: "Ansible docs",
    },
    SeedDomain {
        domain: "javadoc.io",
        strategy: "HttpSimple",
        notes: "Java docs",
    },
    SeedDomain {
        domain: "bbc.com",
        strategy: "HttpSimple",
        notes: "Open access to articles",
    },
    SeedDomain {
        domain: "reuters.com",
        strategy: "HttpSimple",
        notes: "Open news articles",
    },
    SeedDomain {
        domain: "apnews.com",
        strategy: "HttpSimple",
        notes: "Open access",
    },
    SeedDomain {
        domain: "npr.org",
        strategy: "HttpSimple",
        notes: "Open access",
    },
    SeedDomain {
        domain: "theguardian.com",
        strategy: "HttpSimple",
        notes: "Open access",
    },
    SeedDomain {
        domain: "arstechnica.com",
        strategy: "HttpSimple",
        notes: "Tech news",
    },
    SeedDomain {
        domain: "techcrunch.com",
        strategy: "HttpSimple",
        notes: "Tech news",
    },
    SeedDomain {
        domain: "wired.com",
        strategy: "HttpSimple",
        notes: "Conde Nast",
    },
    SeedDomain {
        domain: "theverge.com",
        strategy: "HttpSimple",
        notes: "Vox Media",
    },
    SeedDomain {
        domain: "bloomberg.com",
        strategy: "Headless",
        notes: "Paywall, JS-rendered",
    },
    SeedDomain {
        domain: "nytimes.com",
        strategy: "Headless",
        notes: "Paywall, JS-heavy",
    },
    SeedDomain {
        domain: "github.com",
        strategy: "HttpSimple",
        notes: "Open API-friendly",
    },
    SeedDomain {
        domain: "gitlab.com",
        strategy: "HttpSimple",
        notes: "Open access",
    },
    SeedDomain {
        domain: "bitbucket.org",
        strategy: "HttpSimple",
        notes: "Atlassian, open repos",
    },
    SeedDomain {
        domain: "codeberg.org",
        strategy: "HttpSimple",
        notes: "Gitea-based, open",
    },
    SeedDomain {
        domain: "sourcehut.org",
        strategy: "HttpSimple",
        notes: "Minimal, open",
    },
    SeedDomain {
        domain: "gitee.com",
        strategy: "HttpSimple",
        notes: "Chinese GitHub mirror",
    },
    SeedDomain {
        domain: "stackoverflow.com",
        strategy: "HttpSimple",
        notes: "Open access, good HTML",
    },
    SeedDomain {
        domain: "stackexchange.com",
        strategy: "HttpSimple",
        notes: "Open access",
    },
    SeedDomain {
        domain: "superuser.com",
        strategy: "HttpSimple",
        notes: "StackExchange network",
    },
    SeedDomain {
        domain: "askubuntu.com",
        strategy: "HttpSimple",
        notes: "StackExchange network",
    },
    SeedDomain {
        domain: "serverfault.com",
        strategy: "HttpSimple",
        notes: "StackExchange network",
    },
    SeedDomain {
        domain: "hackernews.com",
        strategy: "PublicApi",
        notes: "HN has public API",
    },
    SeedDomain {
        domain: "news.ycombinator.com",
        strategy: "PublicApi",
        notes: "HN official domain",
    },
    SeedDomain {
        domain: "youtube.com",
        strategy: "PublicApi",
        notes: "noembed metadata API",
    },
    SeedDomain {
        domain: "www.youtube.com",
        strategy: "PublicApi",
        notes: "YouTube www subdomain",
    },
    SeedDomain {
        domain: "youtu.be",
        strategy: "PublicApi",
        notes: "YouTube short URL",
    },
    SeedDomain {
        domain: "m.youtube.com",
        strategy: "PublicApi",
        notes: "YouTube mobile URL",
    },
    SeedDomain {
        domain: "crates.io",
        strategy: "HttpSimple",
        notes: "Rust crate registry",
    },
    SeedDomain {
        domain: "npmjs.com",
        strategy: "HttpSimple",
        notes: "npm registry",
    },
    SeedDomain {
        domain: "pypi.org",
        strategy: "HttpSimple",
        notes: "Python package index",
    },
    SeedDomain {
        domain: "hub.docker.com",
        strategy: "HttpSimple",
        notes: "Docker Hub",
    },
    SeedDomain {
        domain: "rubygems.org",
        strategy: "HttpSimple",
        notes: "Ruby gem registry",
    },
];

pub fn import_seeds(cache: &crate::storage::route_cache::RouteCache) -> anyhow::Result<u64> {
    let mut count = 0u64;
    for seed in SEED_DOMAINS {
        let result = crate::storage::route_cache::FetchResult {
            latency_ms: 500,
            success: true,
            headers: None,
        };
        cache.upsert_strategy(seed.domain, seed.strategy, &result)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_domain_count() {
        assert!(
            SEED_DOMAINS.len() >= 40,
            "Expected at least 40 seed domains, got {}",
            SEED_DOMAINS.len()
        );
    }

    #[test]
    fn no_duplicate_domains() {
        let mut seen = std::collections::HashSet::new();
        for d in SEED_DOMAINS {
            assert!(seen.insert(d.domain), "Duplicate seed domain: {}", d.domain);
        }
    }

    #[test]
    fn strategies_are_valid_names() {
        let valid = [
            "HttpSimple",
            "TlsChrome",
            "TlsMobile",
            "Headless",
            "PublicApi",
        ];
        for d in SEED_DOMAINS {
            assert!(
                valid.contains(&d.strategy),
                "Invalid strategy '{}' for domain '{}'",
                d.strategy,
                d.domain
            );
        }
    }

    #[test]
    fn test_import_seeds() {
        let storage = crate::storage::Storage::open(":memory:").unwrap();
        let cache = crate::storage::route_cache::RouteCache::new(&storage);
        let count = import_seeds(&cache).expect("import_seeds should succeed");
        assert_eq!(count, SEED_DOMAINS.len() as u64);
        assert_eq!(cache.count_entries().unwrap(), SEED_DOMAINS.len() as u64);
    }
}
