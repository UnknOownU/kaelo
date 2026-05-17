use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct RobotsRules {
    pub user_agent: String,
    pub disallow: Vec<String>,
    pub allow: Vec<String>,
}

impl RobotsRules {
    pub fn is_path_allowed(&self, path: &str) -> bool {
        // Check disallow rules first (most specific wins)
        let mut longest_disallow = 0usize;
        for pattern in &self.disallow {
            if pattern.is_empty() {
                continue;
            }
            if path_matches(path, pattern) && pattern.len() > longest_disallow {
                longest_disallow = pattern.len();
            }
        }

        // Check allow rules (override disallow if more specific)
        let mut longest_allow = 0usize;
        for pattern in &self.allow {
            if path_matches(path, pattern) && pattern.len() > longest_allow {
                longest_allow = pattern.len();
            }
        }

        // If allow is more specific than disallow, allow it
        if longest_allow >= longest_disallow && longest_allow > 0 {
            return true;
        }

        // If there's a matching disallow, block it
        if longest_disallow > 0 {
            return false;
        }

        // Default: allowed
        true
    }
}

fn path_matches(path: &str, pattern: &str) -> bool {
    // Simple wildcard matching: * matches any characters
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut remaining = path;
        for part in parts.iter() {
            if part.is_empty() {
                continue;
            }
            if let Some(pos) = remaining.find(part) {
                remaining = &remaining[pos + part.len()..];
            } else {
                return false;
            }
        }
        return true;
    }
    // Exact prefix match
    path.starts_with(pattern)
}

pub fn parse_robots_txt(text: &str, target_agent: &str) -> RobotsRules {
    let target_lower = target_agent.to_lowercase();

    let mut specific_rules: Option<RobotsRules> = None;
    let mut wildcard_rules = RobotsRules::default();
    let mut current_agent = String::new();
    let mut current_rules = RobotsRules::default();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(agent) = line.strip_prefix("User-agent:") {
            if !current_agent.is_empty() {
                match current_agent.as_str() {
                    a if a == target_lower => {
                        specific_rules = Some(current_rules.clone());
                    }
                    "*" => {
                        wildcard_rules = current_rules.clone();
                    }
                    _ => {}
                }
            }
            current_agent = agent.trim().to_lowercase();
            current_rules = RobotsRules::default();
            current_rules.user_agent = current_agent.clone();
        } else if !current_agent.is_empty() {
            if let Some(path) = line.strip_prefix("Disallow:") {
                current_rules.disallow.push(path.trim().to_string());
            } else if let Some(path) = line.strip_prefix("Allow:") {
                current_rules.allow.push(path.trim().to_string());
            }
        }
    }
    // Flush the last group
    match current_agent.as_str() {
        a if a == target_lower => {
            specific_rules = Some(current_rules);
        }
        "*" => {
            wildcard_rules = current_rules;
        }
        _ => {}
    }

    specific_rules.unwrap_or(wildcard_rules)
}

pub fn is_allowed(path: &str, robots_text: &str, user_agent: &str) -> bool {
    if robots_text.is_empty() {
        return true;
    }
    let rules = parse_robots_txt(robots_text, user_agent);
    rules.is_path_allowed(path)
}

pub struct RobotsCache {
    cache: Mutex<HashMap<String, (String, Instant)>>,
    ttl: Duration,
}

impl Default for RobotsCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RobotsCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(3600), // 1 hour TTL
        }
    }

    pub fn get(&self, domain: &str) -> Option<String> {
        let cache = self.cache.lock().ok()?;
        let (text, fetched) = cache.get(domain)?;
        if fetched.elapsed() > self.ttl {
            return None;
        }
        Some(text.clone())
    }

    pub fn set(&self, domain: &str, text: String) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(domain.to_string(), (text, Instant::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disallow_all() {
        let robots = "User-agent: *\nDisallow: /";
        assert!(!is_allowed("/anything", robots, "MyBot"));
        assert!(!is_allowed("/", robots, "MyBot"));
    }

    #[test]
    fn test_disallow_specific_path() {
        let robots = "User-agent: *\nDisallow: /private/\nDisallow: /admin";
        assert!(!is_allowed("/private/page", robots, "MyBot"));
        assert!(!is_allowed("/admin/settings", robots, "MyBot"));
        assert!(is_allowed("/public/page", robots, "MyBot"));
    }

    #[test]
    fn test_allow_overrides_disallow() {
        let robots = "User-agent: *\nDisallow: /private/\nAllow: /private/public/";
        assert!(!is_allowed("/private/secret", robots, "MyBot"));
        assert!(is_allowed("/private/public/doc", robots, "MyBot"));
    }

    #[test]
    fn test_empty_robots_allows_all() {
        assert!(is_allowed("/anything", "", "MyBot"));
    }

    #[test]
    fn test_specific_user_agent() {
        let robots = "User-agent: BadBot\nDisallow: /\n\nUser-agent: *\nAllow: /";
        assert!(!is_allowed("/page", robots, "BadBot"));
        assert!(is_allowed("/page", robots, "GoodBot"));
    }

    #[test]
    fn test_wildcard_pattern() {
        let robots = "User-agent: *\nDisallow: /private/*/secret";
        assert!(!is_allowed("/private/xyz/secret", robots, "MyBot"));
        assert!(is_allowed("/private/xyz/public", robots, "MyBot"));
    }

    #[test]
    fn test_cache_set_get() {
        let cache = RobotsCache::new();
        cache.set("example.com", "User-agent: *\nDisallow: /".to_string());
        let text = cache.get("example.com").unwrap();
        assert!(text.contains("Disallow"));
    }
}
