use glob::glob;
use serde::{Deserialize, Serialize};
use std::fs::File;
use zip::ZipArchive;

#[derive(Debug, Clone, PartialEq, Eq,Serialize, Deserialize)]
pub struct MavenCoordinates {
    #[serde(rename = "groupId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(rename = "artifactId")]
    #[serde(skip_serializing_if = "String::is_empty")]
    #[serde(default)]
    pub artifact_id: String,
    #[serde(rename = "version")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "classifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier: Option<String>,
    #[serde(rename = "packaging")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packaging: Option<String>,
    #[serde(rename = "jarPath")]
    #[serde(skip_serializing)]
    pub jar_path: Option<String>,
}

impl MavenCoordinates {
    pub fn new(
        group_id: Option<String>,
        artifact_id: String,
        version: Option<String>,
        classifier: Option<String>,
        packaging: Option<String>,
    ) -> Self {
        MavenCoordinates {
            group_id,
            artifact_id,
            version,
            classifier,
            packaging,
            jar_path: None,
        }
    }

    pub fn read_from_path(path: &str) -> Result<Vec<MavenCoordinates>, anyhow::Error> {
        let mut coordinates = Vec::new();
        let glob_result = match glob(path) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("Warning: Invalid glob pattern '{}': {}", path, e);
                return Ok(coordinates);
            }
        };
        for path in glob_result {
            match path {
                Ok(path) => {
                    if path.is_dir() && path.exists() {
                        let entries = match std::fs::read_dir(&path) {
                            Ok(e) => e,
                            Err(e) => {
                                eprintln!("Warning: Failed to read directory {:?}: {}", path, e);
                                continue;
                            }
                        };
                        for entry in entries {
                            let entry = match entry {
                                Ok(e) => e,
                                Err(e) => {
                                    eprintln!("Warning: Failed to read entry: {}", e);
                                    continue;
                                }
                            };
                            let entry_path = entry.path();
                            if entry_path.is_file() {
                                let file_name = match entry_path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                {
                                    Some(name) => name,
                                    None => {
                                        eprintln!("Warning: Non-UTF8 file name: {:?}", entry_path);
                                        continue;
                                    }
                                };
                                if file_name.ends_with(".jar") {
                                    println!("Find Jar File: {:?}", file_name);
                                    let path_str = match entry_path.to_str() {
                                        Some(s) => s.to_string(),
                                        None => {
                                            eprintln!("Warning: Non-UTF8 path: {:?}", entry_path);
                                            continue;
                                        }
                                    };
                                    match MavenCoordinates::from_jar(&path_str) {
                                        Ok(jars) => {
                                            for jar in jars {
                                                coordinates.push(jar);
                                            }
                                        }
                                        Err(e) => eprintln!(
                                            "Warning: Failed to parse JAR {:?}: {}",
                                            path_str, e
                                        ),
                                    }
                                }
                            }
                        }
                    } else if path.is_file() {
                        let path_str = match path.to_str() {
                            Some(s) => s,
                            None => {
                                eprintln!("Warning: Non-UTF8 path: {:?}", path);
                                continue;
                            }
                        };
                        if path_str.ends_with(".jar") {
                            match MavenCoordinates::from_jar(path_str) {
                                Ok(jars) => {
                                    for jar in jars {
                                        coordinates.push(jar);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Warning: Failed to parse JAR {:?}: {}", path_str, e)
                                }
                            }
                        }
                    } else {
                        eprintln!("Path is not a jar file or directory: {:?}", path);
                    }
                }
                Err(e) => eprintln!("Error: {:?}", e),
            }
        }
        Ok(coordinates)
    }

    pub fn from_jar(jar_path: &str) -> Result<Vec<MavenCoordinates>, anyhow::Error> {
        let abs_jar_path = std::fs::canonicalize(jar_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| jar_path.to_string());
        let file = File::open(jar_path)?;
        let mut archive = ZipArchive::new(file)?;
        let mut coordinates = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let path = entry.name().to_string();

            // 匹配Maven元数据文件路径
            if path.starts_with("META-INF/maven/") && (path.ends_with("pom.properties")) {
                let mut content = String::new();
                use std::io::Read;
                entry.read_to_string(&mut content)?;
                if let Some(mut maven_coordinates) = parse_properties(&content) {
                    maven_coordinates.jar_path = Some(abs_jar_path.clone());
                    coordinates.push(maven_coordinates);
                }
            }
        }
        if coordinates.len() > 1 {
            // uber/shaded jar：多个pom.properties，只保留与jar文件名匹配的主坐标
            let file_name = abs_jar_path.split('/').last().unwrap_or(&abs_jar_path);
            let file_stem = file_name.strip_suffix(".jar").unwrap_or(file_name);
            let expected_artifact_id = extract_artifact_id_from_filename(file_stem);
            if let Some(ref aid) = expected_artifact_id {
                if let Some(primary) = coordinates.iter().find(|c| c.artifact_id == *aid) {
                    coordinates = vec![primary.clone()];
                }
            }
        }

        if coordinates.is_empty() {
            // 根据文件名推测Maven坐标
            let file_name = abs_jar_path.split('/').last().unwrap_or(&abs_jar_path);
            if !file_name.ends_with(".jar") {
                return Err(anyhow::anyhow!("Not a JAR file"));
            }
            let file_name = file_name.strip_suffix(".jar").unwrap_or(file_name);
            let parts: Vec<&str> = file_name.split('-').collect();
            if parts.len() < 2 {
                let mut jar = MavenCoordinates::new(None, file_name.to_string(), None, None, None);
                jar.jar_path = Some(abs_jar_path.clone());
                coordinates.push(jar.clone());
                return Ok(coordinates);
            }
            //zstd-jni-1.5.6-3
            // 查询第一个前面是'-'的数字的位置
            let mut version_pos = 0;
            let chars: Vec<char> = file_name.chars().collect();
            for i in 0..file_name.len() {
                if chars[i] == '-' && i < chars.len() - 1 && chars[i + 1].is_digit(10) {
                    version_pos = i;
                    break;
                }
            }

            let version_str = if version_pos > 0 {
                &file_name[version_pos + 1..]
            } else {
                ""
            };
            if version_pos == 0 {
                let mut jar =
                    MavenCoordinates::new(None, file_name.to_string(), None, None, None);
                jar.jar_path = Some(abs_jar_path.clone());
                coordinates.push(jar.clone());
                return Ok(coordinates);
            }

            let artifact_id = file_name[0..version_pos].to_string();
            let version = Some(version_str.to_string());
            let mut jar = MavenCoordinates::new(None, artifact_id, version, None, None);
            jar.jar_path = Some(abs_jar_path.clone());
            coordinates.push(jar.clone());
            return Ok(coordinates.clone());
        } else {
            return Ok(coordinates.clone());
        }
    }
    pub fn to_string(&self) -> String {
        let mut result = String::new();
        if let Some(ref group_id) = self.group_id {
            result.push_str(&format!("{}:", group_id));
        }
        result.push_str(&self.artifact_id);
        if let Some(ref version) = self.version {
            result.push_str(&format!(":{}", version));
        }
        if let Some(ref classifier) = self.classifier {
            result.push_str(&format!("-{}", classifier));
        }
        if let Some(ref packaging) = self.packaging {
            result.push_str(&format!(".{}", packaging));
        }
        result
    }
}
fn extract_artifact_id_from_filename(file_stem: &str) -> Option<String> {
    let chars: Vec<char> = file_stem.chars().collect();
    for i in 0..file_stem.len() {
        if chars[i] == '-' && i < chars.len() - 1 && chars[i + 1].is_ascii_digit() {
            return Some(file_stem[0..i].to_string());
        }
    }
    None
}

fn parse_properties(content: &str) -> Option<MavenCoordinates> {
    let mut group_id = None;
    let mut artifact_id = String::new();
    let mut version = None;
    let mut classifier = None;
    let mut packaging = None;
    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "groupId" => group_id = Some(value.trim().to_string()),
                "artifactId" => artifact_id = value.trim().to_string(),
                "version" => version = Some(value.trim().to_string()),
                "classifier" => classifier = Some(value.trim().to_string()),
                "packaging" => packaging = Some(value.trim().to_string()),
                _ => (),
            }
        }
    }
    if artifact_id.is_empty() {
        return None;
    }
    Some(MavenCoordinates {
        group_id,
        artifact_id,
        version,
        classifier,
        packaging,
        jar_path: None,
    })
}

impl std::fmt::Display for MavenCoordinates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_string())
    }
}
impl std::str::FromStr for MavenCoordinates {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() < 2 {
            return Err("Invalid Maven coordinates".to_string());
        }

        let (group_id, artifact_id, version) = if parts.len() == 2 {
            // Format: artifactId:version
            (None, parts[0].to_string(), Some(parts[1].to_string()))
        } else {
            // Format: groupId:artifactId:version[:classifier][:packaging]
            (
                Some(parts[0].to_string()),
                parts[1].to_string(),
                Some(parts[2].to_string()),
            )
        };

        let classifier = if parts.len() > 3 {
            Some(parts[3].to_string())
        } else {
            None
        };
        let packaging = if parts.len() > 4 {
            Some(parts[4].to_string())
        } else {
            None
        };
        Ok(MavenCoordinates {
            group_id,
            artifact_id,
            version,
            classifier,
            packaging,
            jar_path: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_maven_coordinates() {
        let coordinates = MavenCoordinates::new(
            Some("com.example".to_string()),
            "example-artifact".to_string(),
            Some("1.0.0".to_string()),
            Some("jar".to_string()),
            Some("jar".to_string()),
        );
        assert_eq!(coordinates.group_id, Some("com.example".to_string()));
        assert_eq!(coordinates.artifact_id, "example-artifact".to_string());
    }
    #[test]
    fn test_parse_properties() {
        let content = "groupId=com.example\nartifactId=example-artifact\nversion=1.0.0\nclassifier=jar\npackaging=jar";
        let coordinates = parse_properties(content).unwrap();
        assert_eq!(coordinates.group_id, Some("com.example".to_string()));
        assert_eq!(coordinates.artifact_id, "example-artifact".to_string());
        assert_eq!(coordinates.version, Some("1.0.0".to_string()));
        assert_eq!(coordinates.classifier, Some("jar".to_string()));
        assert_eq!(coordinates.packaging, Some("jar".to_string()));
    }
    #[test]
    fn test_parse_jar() {
        let jar_path =
            "/Users/zml/Workspace/servers/apache-tomcat-9.0.41/webapps/flowable-rest/WEB-INF/lib/";
        let coordinates = MavenCoordinates::read_from_path(jar_path).unwrap();
        println!("{:?}", serde_json::to_string(&coordinates).unwrap());
    }
}
