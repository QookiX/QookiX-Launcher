//! Maven 元数据中的版本列表和排序规则。

/// Parse a maven-metadata.xml `<versions>` block without an XML dependency.
pub fn parse_maven_versions(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<version>") {
        let after = &rest[start + "<version>".len()..];
        if let Some(end) = after.find("</version>") {
            out.push(after[..end].to_string());
            rest = &after[end + "</version>".len()..];
        } else {
            break;
        }
    }
    out
}

/// Sort version strings that look like `1.20.1-47.2.0` (mc-build) descending.
pub fn sort_mc_versions(mut versions: Vec<String>, mc_prefix: &str) -> Vec<String> {
    versions.retain(|v| v.starts_with(mc_prefix));
    versions.sort_by(|a, b| {
        let num = |s: &str| -> Vec<u64> {
            let part = s.split('-').last().unwrap_or(s);
            part.split('.')
                .filter_map(|p| p.parse::<u64>().ok())
                .collect()
        };
        num(b).partial_cmp(&num(a)).unwrap_or(std::cmp::Ordering::Equal)
    });
    versions
}

/// Sort arbitrary version strings (e.g. `0.15.11`, `20.4.237`) descending
/// by their numeric components.
pub fn sort_version_desc(mut versions: Vec<String>) -> Vec<String> {
    versions.sort_by(|a, b| {
        let num = |s: &str| -> Vec<u64> {
            s.split(|c: char| !c.is_ascii_digit())
                .filter(|p| !p.is_empty())
                .filter_map(|p| p.parse().ok())
                .collect()
        };
        num(b).partial_cmp(&num(a)).unwrap_or(std::cmp::Ordering::Equal)
    });
    versions
}
