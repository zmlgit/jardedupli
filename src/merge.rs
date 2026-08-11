use crate::maven_coordinate::MavenCoordinates;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum Strategy {
    Source,
    Target,
}

fn make_key(coord: &MavenCoordinates) -> String {
    if let Some(group_id) = &coord.group_id {
        format!("{}:{}", group_id, coord.artifact_id)
    } else {
        format!("{}", coord.artifact_id)
    }
}

/// 合并 source 和 target 的 jar 坐标。
///
/// 同 key 同版本：按 strategy 选 primary（source=新构建覆盖部署 / target=保留现有部署）。
/// 同 key 多版本冲突：whitelist 命中且版本存在候选 → 用 whitelist 版本；否则按 strategy fallback。
pub(crate) fn merge_jars(
    sources: Vec<MavenCoordinates>,
    targets: Vec<MavenCoordinates>,
    management: Option<Vec<MavenCoordinates>>,
    strategy: Strategy,
) -> anyhow::Result<BTreeMap<String, MavenCoordinates>> {
    let mut source_map: HashMap<String, Vec<MavenCoordinates>> = HashMap::new();
    for s in sources {
        source_map.entry(make_key(&s)).or_default().push(s);
    }
    let mut target_map: HashMap<String, Vec<MavenCoordinates>> = HashMap::new();
    for t in targets {
        target_map.entry(make_key(&t)).or_default().push(t);
    }

    let mut whitelist_map: HashMap<String, MavenCoordinates> = HashMap::new();
    if let Some(management) = management {
        for item in management {
            let key = make_key(&item);
            if whitelist_map.contains_key(&key) {
                eprintln!("Warning: duplicate whitelist key '{}', keeping first entry", key);
            } else {
                whitelist_map.insert(key, item);
            }
        }
    }

    let prefer_source = matches!(strategy, Strategy::Source);

    let all_keys: std::collections::HashSet<String> = source_map
        .keys()
        .chain(target_map.keys())
        .cloned()
        .collect();

    let mut merged: BTreeMap<String, MavenCoordinates> = BTreeMap::new();

    for key in all_keys {
        let srcs = source_map.get(&key).cloned().unwrap_or_default();
        let tgts = target_map.get(&key).cloned().unwrap_or_default();

        let mut all: Vec<&MavenCoordinates> = Vec::new();
        for s in &srcs {
            all.push(s);
        }
        for t in &tgts {
            all.push(t);
        }

        let unique_versions: std::collections::HashSet<&str> = all
            .iter()
            .map(|c| c.version.as_deref().unwrap_or(""))
            .collect();

        let primary_from_whitelist = if unique_versions.len() > 1 {
            if let Some(wl) = whitelist_map.get(&key) {
                match all.iter().find(|c| c.version == wl.version).copied() {
                    Some(m) => Some(m),
                    None => {
                        eprintln!(
                            "Warning: whitelist version '{}' not found for '{}', falling back to {}",
                            wl.version.as_deref().unwrap_or("?"),
                            key,
                            if prefer_source { "source" } else { "target" }
                        );
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let primary = primary_from_whitelist.or_else(|| {
            let first_source = srcs.first();
            let first_target = tgts.first();
            if prefer_source {
                first_source.or(first_target)
            } else {
                first_target.or(first_source)
            }
        });

        let Some(primary) = primary else { continue };

        let jar_path = if primary.jar_path.is_some() {
            primary.jar_path.clone()
        } else {
            all.iter().find_map(|c| c.jar_path.clone())
        };

        let mut entry = primary.clone();
        entry.jar_path = jar_path;
        merged.insert(key.clone(), entry);
    }

    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_jars_source_wins_on_conflict_without_whitelist() {
        let source = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let target = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"2.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let target: Vec<MavenCoordinates> = serde_json::from_str(target).unwrap();
        let result = merge_jars(source, target, None, Strategy::Source).unwrap();
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_merge_jars_target_wins_when_strategy_target() {
        let source = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let target = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"2.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let target: Vec<MavenCoordinates> = serde_json::from_str(target).unwrap();
        let result = merge_jars(source, target, None, Strategy::Target).unwrap();
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("2.0.0".to_string())
        );
    }

    #[test]
    fn test_merge_jars_whitelist_overrides_conflict() {
        let source = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"2.0.0\"}]";
        let target = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let management =
            "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let target: Vec<MavenCoordinates> = serde_json::from_str(target).unwrap();
        let management: Vec<MavenCoordinates> = serde_json::from_str(management).unwrap();
        let result = merge_jars(source, target, Some(management), Strategy::Source).unwrap();
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_merge_jars_jar_path_inherited_from_target_when_source_missing() {
        let source_coord = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        let mut target_coord = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        target_coord.jar_path = Some("/path/to/lib.jar".into());
        let result =
            merge_jars(vec![source_coord], vec![target_coord], None, Strategy::Source).unwrap();
        let merged = result.get("com.example:lib").unwrap();
        assert_eq!(merged.version, Some("1.0.0".to_string()));
        assert_eq!(merged.jar_path, Some("/path/to/lib.jar".to_string()));
    }

    #[test]
    fn test_merge_jars_target_only_preserved() {
        let source = "[]";
        let target = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let target: Vec<MavenCoordinates> = serde_json::from_str(target).unwrap();
        let result = merge_jars(source, target, None, Strategy::Source).unwrap();
        assert!(result.contains_key("com.example:lib"));
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_merge_jars_whitelist_inherits_jar_path_from_matched_version() {
        let mut source_coord = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("2.0.0".into()),
            None,
            None,
        );
        source_coord.jar_path = Some("/path/to/lib-2.0.0.jar".into());
        let mut target_coord = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        target_coord.jar_path = Some("/path/to/lib-1.0.0.jar".into());
        let management_coord = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        let result = merge_jars(
            vec![source_coord],
            vec![target_coord],
            Some(vec![management_coord]),
            Strategy::Source,
        )
        .unwrap();
        let merged = result.get("com.example:lib").unwrap();
        assert_eq!(merged.version, Some("1.0.0".to_string()));
        assert_eq!(
            merged.jar_path,
            Some("/path/to/lib-1.0.0.jar".to_string())
        );
    }

    #[test]
    fn test_merge_jars_duplicate_source_keys_first_wins() {
        let source = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"},{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"2.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let result = merge_jars(source, vec![], None, Strategy::Source).unwrap();
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_whitelist_picks_correct_jar_from_duplicates() {
        let mut v1 = MavenCoordinates::new(
            Some("com.fasterxml.jackson.core".into()),
            "jackson-databind".into(),
            Some("2.12.2".into()),
            None,
            None,
        );
        v1.jar_path = Some("/libs/jackson-databind-2.12.2.jar".into());

        let mut v2 = MavenCoordinates::new(
            Some("com.fasterxml.jackson.core".into()),
            "jackson-databind".into(),
            Some("2.9.9.3".into()),
            None,
            None,
        );
        v2.jar_path = Some("/libs/jackson-databind-2.9.9.3.jar".into());

        let whitelist_entry = MavenCoordinates::new(
            Some("com.fasterxml.jackson.core".into()),
            "jackson-databind".into(),
            Some("2.9.9.3".into()),
            None,
            None,
        );

        let result = merge_jars(
            vec![v1, v2],
            vec![],
            Some(vec![whitelist_entry]),
            Strategy::Source,
        )
        .unwrap();

        let merged = result
            .get("com.fasterxml.jackson.core:jackson-databind")
            .unwrap();
        assert_eq!(merged.version, Some("2.9.9.3".to_string()));
        assert_eq!(
            merged.jar_path,
            Some("/libs/jackson-databind-2.9.9.3.jar".to_string())
        );
    }

    #[test]
    fn test_snapshot_same_version_strategy_source_picks_source_path() {
        let mut src = MavenCoordinates::new(
            Some("com.iss.cms.fsmc".into()),
            "cms.fsmc.impl".into(),
            Some("0.0.1-SNAPSHOT".into()),
            None,
            None,
        );
        src.jar_path = Some("/build/cms.fsmc.impl-0.0.1-SNAPSHOT.jar".into());

        let mut tgt = MavenCoordinates::new(
            Some("com.iss.cms.fsmc".into()),
            "cms.fsmc.impl".into(),
            Some("0.0.1-SNAPSHOT".into()),
            None,
            None,
        );
        tgt.jar_path = Some("/deploy/lib/cms.fsmc.impl-0.0.1-SNAPSHOT.jar".into());

        let result = merge_jars(vec![src.clone()], vec![tgt], None, Strategy::Source).unwrap();
        let merged = result.get("com.iss.cms.fsmc:cms.fsmc.impl").unwrap();
        assert_eq!(merged.version, Some("0.0.1-SNAPSHOT".to_string()));
        assert_eq!(
            merged.jar_path.as_deref(),
            Some("/build/cms.fsmc.impl-0.0.1-SNAPSHOT.jar")
        );
    }

    #[test]
    fn test_snapshot_same_version_strategy_target_keeps_target_path() {
        let mut src = MavenCoordinates::new(
            Some("com.iss.cms.fsmc".into()),
            "cms.fsmc.impl".into(),
            Some("0.0.1-SNAPSHOT".into()),
            None,
            None,
        );
        src.jar_path = Some("/build/cms.fsmc.impl-0.0.1-SNAPSHOT.jar".into());

        let mut tgt = MavenCoordinates::new(
            Some("com.iss.cms.fsmc".into()),
            "cms.fsmc.impl".into(),
            Some("0.0.1-SNAPSHOT".into()),
            None,
            None,
        );
        tgt.jar_path = Some("/deploy/lib/cms.fsmc.impl-0.0.1-SNAPSHOT.jar".into());

        let result = merge_jars(vec![src], vec![tgt.clone()], None, Strategy::Target).unwrap();
        let merged = result.get("com.iss.cms.fsmc:cms.fsmc.impl").unwrap();
        assert_eq!(
            merged.jar_path.as_deref(),
            Some("/deploy/lib/cms.fsmc.impl-0.0.1-SNAPSHOT.jar")
        );
    }
}
