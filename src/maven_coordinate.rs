use glob::glob;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use zip::ZipArchive;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    pub fn read_from_json_file(path: &str) -> Result<Vec<MavenCoordinates>, anyhow::Error> {
        let mut file = File::open(path)?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)?;
        let jars = serde_json::from_str::<Vec<MavenCoordinates>>(&buf)
            .map_err(|e| anyhow::anyhow!("Failed to parse JSON in {}: {}", path, e))?;
        Ok(jars)
    }

    pub fn read_from_path(paths: &[String]) -> Result<Vec<MavenCoordinates>, anyhow::Error> {
        let mut coordinates = Vec::new();
        for raw in paths {
            let expanded = shellexpand_home(raw);
            let glob_iter = match glob(&expanded) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("Warning: invalid glob pattern '{}': {}", expanded, e);
                    continue;
                }
            };
            for entry in glob_iter {
                let path = match entry {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Warning: glob entry error: {}", e);
                        continue;
                    }
                };
                if path.is_dir() {
                    Self::scan_dir(&path, &mut coordinates);
                } else if path.is_file() && path.extension().is_some_and(|e| e == "jar") {
                    Self::scan_jar_file(&path, &mut coordinates);
                }
            }
        }
        Ok(coordinates)
    }

    fn scan_dir(dir: &std::path::Path, out: &mut Vec<MavenCoordinates>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Warning: failed to read directory {:?}: {}", dir, e);
                return;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "jar") {
                Self::scan_jar_file(&path, out);
            }
        }
    }

    fn scan_jar_file(path: &std::path::Path, out: &mut Vec<MavenCoordinates>) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            eprintln!("Warning: non-UTF8 file name: {:?}", path);
            return;
        };
        println!("Find Jar File: {}", name);
        let Some(path_str) = path.to_str() else {
            eprintln!("Warning: non-UTF8 path: {:?}", path);
            return;
        };
        match Self::from_jar(path_str) {
            Ok(jars) => out.extend(jars),
            Err(e) => eprintln!("Warning: failed to parse JAR {:?}: {}", path_str, e),
        }
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
            if path.starts_with("META-INF/maven/") && path.ends_with("pom.properties") {
                let mut content = String::new();
                use std::io::Read;
                entry.read_to_string(&mut content)?;
                if let Some(mut c) = parse_properties(&content) {
                    c.jar_path = Some(abs_jar_path.clone());
                    coordinates.push(c);
                }
            }
        }
        if coordinates.len() > 1 {
            // uber/shaded jar：保留与 jar 文件名匹配的主坐标，避免一个文件被当成多个独立 artifact
            let file_name = abs_jar_path.split('/').next_back().unwrap_or(&abs_jar_path);
            let stem = file_name.strip_suffix(".jar").unwrap_or(file_name);
            if let Some(aid) = extract_artifact_id(stem) {
                if let Some(primary) = coordinates.iter().find(|c| c.artifact_id == aid) {
                    coordinates = vec![primary.clone()];
                }
            }
        }

        if !coordinates.is_empty() {
            return Ok(coordinates);
        }

        // 没有 pom.properties：按文件名推测
        let file_name = abs_jar_path.split('/').next_back().unwrap_or(&abs_jar_path);
        let stem = match file_name.strip_suffix(".jar") {
            Some(s) => s,
            None => return Err(anyhow::anyhow!("Not a JAR file: {}", jar_path)),
        };
        let (artifact_id, version) = split_artifact_version(stem);
        let mut jar = MavenCoordinates::new(None, artifact_id, version, None, None);
        jar.jar_path = Some(abs_jar_path);
        Ok(vec![jar])
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

fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let mut s = home.to_string_lossy().into_owned();
            s.push('/');
            s.push_str(rest);
            return s;
        }
    } else if p == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return home.to_string_lossy().into_owned();
        }
    }
    p.to_string()
}

fn extract_artifact_id(stem: &str) -> Option<String> {
    for (i, c) in stem.char_indices() {
        if c == '-' {
            let after = stem[i + c.len_utf8()..].chars().next();
            if after.is_some_and(|a| a.is_ascii_digit()) {
                return Some(stem[..i].to_string());
            }
        }
    }
    None
}

fn split_artifact_version(stem: &str) -> (String, Option<String>) {
    for (i, c) in stem.char_indices() {
        if c == '-' {
            let after = stem[i + c.len_utf8()..].chars().next();
            if after.is_some_and(|a| a.is_ascii_digit()) {
                let artifact = stem[..i].to_string();
                let version = stem[i + c.len_utf8()..].to_string();
                return (artifact, Some(version));
            }
        }
    }
    (stem.to_string(), None)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_jar(path: &std::path::Path, props: &[(&str, &str)]) -> std::io::Result<()> {
        let file = File::create(path)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (entry_path, content) in props {
            zip.start_file(entry_path, opts)?;
            zip.write_all(content.as_bytes())?;
        }
        zip.finish()?;
        Ok(())
    }

    #[test]
    fn test_maven_coordinates_new() {
        let c = MavenCoordinates::new(
            Some("com.example".into()),
            "example-artifact".into(),
            Some("1.0.0".into()),
            Some("sources".into()),
            Some("jar".into()),
        );
        assert_eq!(c.group_id.as_deref(), Some("com.example"));
        assert_eq!(c.artifact_id, "example-artifact");
        assert_eq!(c.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn test_read_from_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wl.json");
        std::fs::write(
            &path,
            r#"[{"groupId":"io.netty","artifactId":"netty-common","version":"4.1.113.Final"},{"artifactId":"spring-core","version":"5.1.10.RELEASE"}]"#,
        )
        .unwrap();
        let v = MavenCoordinates::read_from_json_file(path.to_str().unwrap()).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].group_id.as_deref(), Some("io.netty"));
        assert_eq!(v[0].artifact_id, "netty-common");
        assert!(v[1].group_id.is_none());
        assert_eq!(v[1].artifact_id, "spring-core");
    }

    #[test]
    fn test_read_from_json_file_invalid_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(MavenCoordinates::read_from_json_file(path.to_str().unwrap()).is_err());
    }

    #[test]
    fn test_read_from_json_file_missing_returns_err() {
        assert!(MavenCoordinates::read_from_json_file("/tmp/jardedupli_does_not_exist_xyz.json").is_err());
    }

    #[test]
    fn test_parse_properties_full() {
        let content = "groupId=com.example\nartifactId=example-artifact\nversion=1.0.0\nclassifier=jar\npackaging=jar";
        let c = parse_properties(content).unwrap();
        assert_eq!(c.group_id.as_deref(), Some("com.example"));
        assert_eq!(c.artifact_id, "example-artifact");
        assert_eq!(c.version.as_deref(), Some("1.0.0"));
        assert_eq!(c.classifier.as_deref(), Some("jar"));
        assert_eq!(c.packaging.as_deref(), Some("jar"));
    }

    #[test]
    fn test_parse_properties_missing_artifact_returns_none() {
        assert!(parse_properties("groupId=g\nversion=1").is_none());
        assert!(parse_properties("").is_none());
    }

    #[test]
    fn test_from_jar_reads_pom_properties() {
        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("my-lib-1.2.3.jar");
        let props = "#Created by Apache Maven 3.6.3\ngroupId=com.example\nartifactId=my-lib\nversion=1.2.3\n";
        make_jar(
            &jar_path,
            &[("META-INF/maven/com.example/my-lib/pom.properties", props)],
        )
        .unwrap();
        let coords = MavenCoordinates::from_jar(jar_path.to_str().unwrap()).unwrap();
        assert_eq!(coords.len(), 1);
        let c = &coords[0];
        assert_eq!(c.group_id.as_deref(), Some("com.example"));
        assert_eq!(c.artifact_id, "my-lib");
        assert_eq!(c.version.as_deref(), Some("1.2.3"));
        assert!(c.jar_path.is_some());
    }

    #[test]
    fn test_from_jar_falls_back_to_filename() {
        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("awesome-lib-2.0.jar");
        // 空 zip，没有 pom.properties
        make_jar(&jar_path, &[]).unwrap();
        let coords = MavenCoordinates::from_jar(jar_path.to_str().unwrap()).unwrap();
        assert_eq!(coords.len(), 1);
        assert_eq!(coords[0].artifact_id, "awesome-lib");
        assert_eq!(coords[0].version.as_deref(), Some("2.0"));
    }

    #[test]
    fn test_from_jar_filename_without_version() {
        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("nolibraryversion.jar");
        make_jar(&jar_path, &[]).unwrap();
        let coords = MavenCoordinates::from_jar(jar_path.to_str().unwrap()).unwrap();
        assert_eq!(coords[0].artifact_id, "nolibraryversion");
        assert!(coords[0].version.is_none());
    }

    #[test]
    fn test_from_jar_multibyte_filename_no_panic() {
        // 非 ASCII 文件名不应导致 byte-index panic
        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("中文库-1.0.jar");
        make_jar(&jar_path, &[]).unwrap();
        let coords = MavenCoordinates::from_jar(jar_path.to_str().unwrap()).unwrap();
        // 不 panic 即通过；artifact_id 推测结果取决于实现
        assert_eq!(coords.len(), 1);
    }

    #[test]
    fn test_from_jar_non_zip_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("not-a-real-jar.jar");
        std::fs::write(&jar_path, b"this is not a zip").unwrap();
        assert!(MavenCoordinates::from_jar(jar_path.to_str().unwrap()).is_err());
    }

    #[test]
    fn test_extract_artifact_id_versioned() {
        assert_eq!(extract_artifact_id("my-lib-1.0"), Some("my-lib".into()));
        assert_eq!(extract_artifact_id("zstd-jni-1.5.6-3"), Some("zstd-jni".into()));
        assert_eq!(extract_artifact_id("noversion"), None);
    }

    #[test]
    fn test_split_artifact_version() {
        let (a, v) = split_artifact_version("my-lib-1.2.3");
        assert_eq!(a, "my-lib");
        assert_eq!(v.as_deref(), Some("1.2.3"));

        let (a, v) = split_artifact_version("plain");
        assert_eq!(a, "plain");
        assert!(v.is_none());
    }

    #[test]
    fn test_shellexpand_home() {
        // Rust 2024: env::set_var 是 unsafe（多线程可能 race）；测试是单线程，安全
        unsafe {
            std::env::set_var("HOME", "/home/test");
        }
        assert_eq!(shellexpand_home("~/foo"), "/home/test/foo");
        assert_eq!(shellexpand_home("~"), "/home/test");
        assert_eq!(shellexpand_home("/abs/path"), "/abs/path");
        assert_eq!(shellexpand_home("relative/path"), "relative/path");
    }
}
