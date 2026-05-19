use std::{collections::HashMap, fs::File, io::Read};

use crate::maven_coordinate::MavenCoordinates;

pub fn read_management_file(management_file: &str) -> Result<Vec<MavenCoordinates>, anyhow::Error> {
    let mut file = File::open(management_file)?;
    let mut buff = String::new();
    file.read_to_string(&mut buff)?;
    let jars = serde_json::from_str::<Vec<MavenCoordinates>>(&buff)
        .map_err(|e| anyhow::anyhow!("Failed to parse JSON: {}", e))?;
    Ok(jars)
}

fn make_key(coord: &MavenCoordinates) -> String {
    if let Some(group_id) = &coord.group_id {
        format!("{}:{}", group_id, coord.artifact_id)
    } else {
        format!("{}", coord.artifact_id)
    }
}

pub(crate) fn merge_jars(
    sources: Vec<MavenCoordinates>,
    targets: Vec<MavenCoordinates>,
    management: Option<Vec<MavenCoordinates>>,
) -> Result<HashMap<String, MavenCoordinates>, anyhow::Error> {
    let mut all_candidates: HashMap<String, Vec<MavenCoordinates>> = HashMap::new();
    for source in sources {
        let key = make_key(&source);
        all_candidates.entry(key).or_default().push(source);
    }
    for target in targets {
        let key = make_key(&target);
        all_candidates.entry(key).or_default().push(target);
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

    let mut merged: HashMap<String, MavenCoordinates> = HashMap::new();

    for (key, candidates) in &all_candidates {
        let unique_versions: std::collections::HashSet<&str> = candidates
            .iter()
            .map(|c| c.version.as_deref().unwrap_or(""))
            .collect();

        let primary = if unique_versions.len() > 1 {
            // 版本冲突：优先用白名单指定的版本
            if let Some(ref wl) = whitelist_map.get(key) {
                let matched = candidates.iter().find(|c| c.version == wl.version);
                if let Some(m) = matched {
                    m
                } else {
                    eprintln!(
                        "Warning: whitelist version '{}' not found for '{}', using default",
                        wl.version.as_deref().unwrap_or("?"),
                        key
                    );
                    candidates.first().unwrap()
                }
            } else {
                candidates.first().unwrap()
            }
        } else {
            // 无冲突：直接用第一个
            candidates.first().unwrap()
        };

        let jar_path = if primary.jar_path.is_some() {
            primary.jar_path.clone()
        } else {
            candidates.iter().find_map(|c| c.jar_path.clone())
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
    fn test_read_management_file() {
        let management_file = "[{\"groupId\":\"io.netty\",\"artifactId\":\"netty-common\",\"version\":\"4.1.113.Final\"},{\"groupId\":\"org.jctools\",\"artifactId\":\"jctools-core\",\"version\":\"4.0.5\"}]";
        let source = "[{\"groupId\":\"io.netty\",\"artifactId\":\"netty-common\",\"version\":\"4.1.113.Final\"},{\"groupId\":\"org.jctoolsss\",\"artifactId\":\"jctools-core\",\"version\":\"4.0.5\"}]";
        let target = "[{\"groupId\":\"io.netty\",\"artifactId\":\"netty-common\",\"version\":\"4.4.113.Final\"},{\"groupId\":\"org.jctoolss\",\"artifactId\":\"jctools-core\",\"version\":\"4.0.5\"}]";
        let management: Vec<MavenCoordinates> = serde_json::from_str(management_file).unwrap();
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let target: Vec<MavenCoordinates> = serde_json::from_str(target).unwrap();
        let result = merge_jars(source, target, Some(management));
        assert!(result.is_ok());
        let jars: HashMap<String, MavenCoordinates> = result.unwrap();
        assert!(!jars.is_empty());
    }

    #[test]
    fn test_merge_jars_source_wins() {
        // Source and target have same key, source should win
        let source = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let target = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"2.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let target: Vec<MavenCoordinates> = serde_json::from_str(target).unwrap();
        let result = merge_jars(source, target, None).unwrap();
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_merge_jars_whitelist_overrides() {
        // Whitelist resolves version conflict
        let source = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"2.0.0\"}]";
        let target = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let management =
            "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let target: Vec<MavenCoordinates> = serde_json::from_str(target).unwrap();
        let management: Vec<MavenCoordinates> = serde_json::from_str(management).unwrap();
        let result = merge_jars(source, target, Some(management)).unwrap();
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_merge_jars_jar_path_inherited() {
        // Source without jar_path should inherit from target
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
            Some("2.0.0".into()),
            None,
            None,
        );
        target_coord.jar_path = Some("/path/to/lib.jar".into());
        let result = merge_jars(vec![source_coord], vec![target_coord], None).unwrap();
        let merged = result.get("com.example:lib").unwrap();
        assert_eq!(merged.version, Some("1.0.0".to_string()));
        assert_eq!(merged.jar_path, Some("/path/to/lib.jar".to_string()));
    }

    #[test]
    fn test_merge_jars_target_only() {
        // Target entry not in source should be preserved
        let source = "[]";
        let target = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let target: Vec<MavenCoordinates> = serde_json::from_str(target).unwrap();
        let result = merge_jars(source, target, None).unwrap();
        assert!(result.contains_key("com.example:lib"));
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_merge_jars_whitelist_inherits_jar_path() {
        // Whitelist resolves conflict, picks matching version's jar_path
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
        let result = merge_jars(vec![source_coord], vec![target_coord], Some(vec![management_coord])).unwrap();
        let merged = result.get("com.example:lib").unwrap();
        // Whitelist version wins
        assert_eq!(merged.version, Some("1.0.0".to_string()));
        // jar_path from the v1.0.0 candidate
        assert_eq!(merged.jar_path, Some("/path/to/lib-1.0.0.jar".to_string()));
    }

    #[test]
    fn test_merge_jars_duplicate_source_warning() {
        // Duplicate source keys: first entry used as primary
        let source = "[{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"1.0.0\"},{\"groupId\":\"com.example\",\"artifactId\":\"lib\",\"version\":\"2.0.0\"}]";
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let result = merge_jars(source, vec![], None).unwrap();
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
        )
        .unwrap();

        let merged = result.get("com.fasterxml.jackson.core:jackson-databind").unwrap();
        assert_eq!(merged.version, Some("2.9.9.3".to_string()));
        assert_eq!(
            merged.jar_path,
            Some("/libs/jackson-databind-2.9.9.3.jar".to_string())
        );
    }
}
