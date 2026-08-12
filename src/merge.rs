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
/// whitelist（仲裁表）命中某个 key 即生效，不依赖版本冲突：
///   - 版本在候选里 → 仲裁，选用 whitelist 指定的版本
///   - 版本不在候选里 → 屏蔽（该 key 不进结果，main 不会 copy，即不部署）
///
/// whitelist 未命中的 key：同 key 同版本按 strategy 选 primary（source=新构建覆盖部署 /
/// target=保留现有部署）；同 key 多版本按 strategy 取其一。
///
/// 注意：屏蔽是软屏蔽——被屏蔽的包不会被打进 merged，故不会被 copy 到 target；
/// 但 target 里若已存在旧文件，不会被删除（main 只 copy 不删）。
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

        // whitelist 优先：命中即生效（不依赖版本冲突）。
        //   版本在候选 → 仲裁选该版本
        //   版本不在候选 → 屏蔽（该 key 不进 merged，不部署）
        // 未命中 whitelist → fallback 到 strategy
        let primary = match whitelist_map.get(&key) {
            Some(wl) => match all.iter().find(|c| c.version == wl.version).copied() {
                Some(matched) => Some(matched),
                None => {
                    println!(
                        "[BLOCK] {} (excluded by whitelist version '{}')",
                        key,
                        wl.version.as_deref().unwrap_or("?")
                    );
                    continue;
                }
            },
            None => {
                let first_source = srcs.first();
                let first_target = tgts.first();
                if prefer_source {
                    first_source.or(first_target)
                } else {
                    first_target.or(first_source)
                }
            }
        };

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

    // ===== 屏蔽语义测试（whitelist 版本不在候选 → 不部署）=====

    #[test]
    fn test_whitelist_excludes_when_version_not_in_candidates() {
        // 冲突场景：whitelist 写一个不存在的版本 → 屏蔽（不再 fallback）
        let mut src = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("2.0.0".into()),
            None,
            None,
        );
        src.jar_path = Some("/build/lib-2.0.0.jar".into());
        let mut tgt = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        tgt.jar_path = Some("/deploy/lib-1.0.0.jar".into());
        let wl = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("9.9.9-notexist".into()), // 故意写错版本 → 屏蔽
            None,
            None,
        );
        let result = merge_jars(vec![src], vec![tgt], Some(vec![wl]), Strategy::Source).unwrap();
        assert!(
            !result.contains_key("com.example:lib"),
            "写错版本号应屏蔽该包"
        );
    }

    #[test]
    fn test_whitelist_excludes_single_version_no_conflict() {
        // 单版本（无冲突）也能被屏蔽：屏蔽不依赖版本冲突
        let mut src = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        src.jar_path = Some("/build/lib-1.0.0.jar".into());
        let wl = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("9.9.9".into()), // 不在候选
            None,
            None,
        );
        let result = merge_jars(vec![src], vec![], Some(vec![wl]), Strategy::Source).unwrap();
        assert!(!result.contains_key("com.example:lib"));
    }

    #[test]
    fn test_whitelist_arbitrate_single_version_matched() {
        // 单版本 + whitelist 版本正好匹配 → 正常仲裁，不误屏蔽
        let mut src = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        src.jar_path = Some("/build/lib-1.0.0.jar".into());
        let wl = MavenCoordinates::new(
            Some("com.example".into()),
            "lib".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        let result = merge_jars(vec![src], vec![], Some(vec![wl]), Strategy::Source).unwrap();
        assert!(result.contains_key("com.example:lib"));
        assert_eq!(
            result.get("com.example:lib").unwrap().version,
            Some("1.0.0".to_string())
        );
    }

    #[test]
    fn test_whitelist_exclude_only_blocks_matched_key() {
        // 多个包：只屏蔽 whitelist 写错版本的，其余正常
        let mut a = MavenCoordinates::new(
            Some("com.example".into()),
            "a".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        a.jar_path = Some("/build/a-1.0.0.jar".into());
        let mut b = MavenCoordinates::new(
            Some("com.example".into()),
            "b".into(),
            Some("1.0.0".into()),
            None,
            None,
        );
        b.jar_path = Some("/build/b-1.0.0.jar".into());
        let wl = MavenCoordinates::new(
            Some("com.example".into()),
            "a".into(),
            Some("9.9.9".into()), // 只屏蔽 a
            None,
            None,
        );
        let result = merge_jars(vec![a, b], vec![], Some(vec![wl]), Strategy::Source).unwrap();
        assert!(!result.contains_key("com.example:a"), "a 应被屏蔽");
        assert!(result.contains_key("com.example:b"), "b 不受影响");
    }
}
