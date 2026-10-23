//! Model paths types.

pub fn sftp_uri(path: &str) -> String {
    format!(
        "sftp:{}",
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    )
}

pub fn path_from_sftp_uri(uri: &str) -> Result<String, String> {
    let path = uri
        .strip_prefix("sftp:")
        .ok_or("SFTP URI must use the sftp: scheme")?;
    normalize_remote_path(path)
}

pub fn normalize_remote_path(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("SFTP path is empty or invalid".to_string());
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            value => components.push(value),
        }
    }
    Ok(format!("/{}", components.join("/")))
}
