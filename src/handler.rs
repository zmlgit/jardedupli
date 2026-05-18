use std::{fs::File, io::Read};

use crate::maven_coordinate::MavenCoordinates;

pub fn read_management_file(management_file: &str) -> Result<Vec<MavenCoordinates>, anyhow::Error> {
    let mut file = File::open(management_file)
        .map_err(|e| anyhow::anyhow!("Failed to open management file: {}", e))?;
    if file.metadata()?.len() == 0 {
        return Ok(vec![]);
    }
    let mut buff = String::new();
    file.read_to_string(&mut buff)?;
    if buff.is_empty() {
        return Ok(vec![]);
    }
    let jars = serde_json::from_str::<Vec<MavenCoordinates>>(&buff)
        .map_err(|e| anyhow::anyhow!("Failed to parse JSON: {}", e))?;
    Ok(jars)
}

pub(crate) fn merge_jars(
    sources: Vec<MavenCoordinates>,
    management: Option<Vec<MavenCoordinates>>,
) -> Result<Vec<MavenCoordinates>, anyhow::Error> {
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
    let mut result = Vec::new();
    for source in sources {
        let key = if let Some(group_id) = &source.group_id {
            format!("{}:{}", group_id, source.artifact_id)
        } else {
            format!("{}", source.artifact_id)
        };
        if let Some(jar) = whitelist_map.get(key.as_str()) {
            if jar.version == source.version {
                result.push(source.clone());
            }
        } else {
            result.push(source.clone());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_management_file() {
        let management_file = "[{\"groupId\":\"io.netty\",\"artifactId\":\"netty-common\",\"version\":\"4.1.113.Final\"},{\"groupId\":\"org.jctools\",\"artifactId\":\"jctools-core\",\"version\":\"4.0.5\"}]";
        let source = "[{\"groupId\":\"io.netty\",\"artifactId\":\"netty-common\",\"version\":\"4.1.113.Final\"},{\"groupId\":\"org.jctoolsss\",\"artifactId\":\"jctools-core\",\"version\":\"4.0.5\"}]";
        let management: Vec<MavenCoordinates> = serde_json::from_str(management_file).unwrap();
        let source: Vec<MavenCoordinates> = serde_json::from_str(source).unwrap();
        let result = merge_jars(source, Some(management));
        assert!(result.is_ok());
        let jars = result.unwrap();
        assert!(!jars.is_empty());
    }
}
