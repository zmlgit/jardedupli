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
        for path in glob(path).unwrap() {
            match path {
                Ok(path) => {
                    if path.is_dir() && path.exists() {
                        // 列出目录下的所有文件
                        let entries = std::fs::read_dir(path).unwrap();
                        for entry in entries {
                            let entry = entry.unwrap();
                            let path = entry.path();
                            if path.is_file() {
                                // 处理文件
                                let file_name = path.file_name().unwrap().to_str().unwrap();
                                if file_name.ends_with(".jar") {
                                    println!("Find Jar File: {:?}", file_name);
                                    // 处理 jar 文件
                                    let jars = MavenCoordinates::from_jar(&path.to_str().unwrap())
                                        .unwrap();
                                    for jar in jars {
                                        coordinates.push(jar);
                                    }
                                }
                            }
                        }
                    } else if path.is_file() && path.to_str().unwrap().ends_with(".jar") {
                        let path = path.to_str().unwrap();
                        let jars = MavenCoordinates::from_jar(&path).unwrap();
                        for jar in jars {
                            coordinates.push(jar);
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
                    maven_coordinates.jar_path = Some(jar_path.to_string());
                    coordinates.push(maven_coordinates);
                }
            }
        }
        if coordinates.is_empty() {
            // 根据文件名推测Maven坐标
            let file_name = jar_path.split('/').last().unwrap_or(jar_path);
            if !file_name.ends_with(".jar") {
                return Err(anyhow::anyhow!("Not a JAR file"));
            }
            let file_name = file_name.trim_end_matches(".jar");
            let parts: Vec<&str> = file_name.split('-').collect();
            if parts.len() < 2 {
                let mut jar = MavenCoordinates::new(None, file_name.to_string(), None, None, None);
                jar.jar_path = Some(jar_path.to_string());
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

            let version = if version_pos > 0 {
                Some(file_name[version_pos + 1..file_name.len()].to_string())
            } else {
                None
            };
            if version_pos == 0 {
                let mut jar =
                    MavenCoordinates::new(None, file_name.to_string(), version, None, None);
                jar.jar_path = Some(jar_path.to_string());
                coordinates.push(jar.clone());
                return Ok(coordinates);
            }
            let artifact_id = file_name[0..version_pos].to_string();
            let mut jar = MavenCoordinates::new(None, artifact_id, version, None, None);
            jar.jar_path = Some(jar_path.to_string());
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
fn parse_properties(content: &str) -> Option<MavenCoordinates> {
    let mut group_id = None;
    let mut artifact_id = String::new();
    let mut version = None;
    let mut classifier = None;
    let mut packaging = None;
    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            match key {
                "groupId" => group_id = Some(value.to_string()),
                "artifactId" => artifact_id = value.to_string(),
                "version" => version = Some(value.to_string()),
                "classifier" => classifier = Some(value.to_string()),
                "packaging" => packaging = Some(value.to_string()),
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

        let group_id = if parts.len() > 2 {
            Some(parts[0].to_string())
        } else {
            None
        };
        let artifact_id = parts[1].to_string();
        let version = if parts.len() > 3 {
            Some(parts[2].to_string())
        } else {
            None
        };
        let classifier = if parts.len() > 4 {
            Some(parts[3].to_string())
        } else {
            None
        };
        let packaging = if parts.len() > 5 {
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

pub fn get_paths(path: &str) -> Result<Vec<String>, anyhow::Error> {
    let mut paths = Vec::new();
    for entry in glob(path)? {
        match entry {
            Ok(path) => {
                if path.is_dir() {
                    let entries = std::fs::read_dir(path)?;
                    for entry in entries {
                        let entry = entry?;
                        let path = entry.path();
                        if path.is_file() {
                            paths.push(path.to_str().unwrap().to_string());
                        }
                    }
                } else if path.is_file() {
                    paths.push(path.to_str().unwrap().to_string());
                }
            }
            Err(e) => eprintln!("Error: {:?}", e),
        }
    }
    Ok(paths)
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
