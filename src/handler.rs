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

pub(crate) fn merge_jars(
    sources: Vec<MavenCoordinates>,
    targets: Vec<MavenCoordinates>,
    management: Option<Vec<MavenCoordinates>>,
) -> Result<HashMap<String, MavenCoordinates>, anyhow::Error> {
    let mut source_map = std::collections::HashMap::new();
    for source in sources {
        let key = if let Some(group_id) = &source.group_id {
            format!("{}:{}", group_id, source.artifact_id)
        } else {
            format!("{}", source.artifact_id)
        };
        source_map.insert(key, source.clone());
    }

    let mut target_map = std::collections::HashMap::new();
    for target in targets {
        let key = if let Some(group_id) = &target.group_id {
            format!("{}:{}", group_id, target.artifact_id)
        } else {
            format!("{}", target.artifact_id)
        };
        target_map.insert(key, target.clone());
    }
    let mut whitelist_map = std::collections::HashMap::new();
    if let Some(management) = management {
        for item in management {
            let key = if let Some(group_id) = &item.group_id {
                format!("{}:{}", group_id, item.artifact_id)
            } else {
                format!("{}", item.artifact_id)
            };
            whitelist_map.insert(key, item.clone());
        }
    }

    let mut merged = std::collections::HashMap::new();

    for (key, source) in target_map.iter() {
        merged.insert(key.clone(), source.clone());
    }

    for (key, source) in source_map.iter() {
        let jar_path = if let Some(jar_path) = &source.jar_path {
            Some(jar_path.clone())
        } else if let Some(target) = merged.get(key) {
            if let Some(target) = &target.jar_path {
                Some(target.clone())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(jar_path) = jar_path {
            let mut source_clone = source.clone();
            source_clone.jar_path = Some(jar_path);
            merged.insert(key.clone(), source_clone);
        } else {
            merged.insert(key.clone(), source.clone());
        }
    }

    for (key, source) in whitelist_map.iter() {
        let jar_path = if let Some(jar_path) = &source.jar_path {
            Some(jar_path.clone())
        } else if let Some(target) = merged.get(key) {
            if source.version == target.version {
                if let Some(target_jar_path) = &target.jar_path {
                    Some(target_jar_path.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some(jar_path) = jar_path {
            let mut source_clone = source.clone();
            source_clone.jar_path = Some(jar_path);
            merged.insert(key.clone(), source_clone);
        } else {
            merged.insert(key.clone(), source.clone());
        }
        
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
}
