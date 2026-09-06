use std::path::Path;

use crate::routes::album::STAGING_DIRECTORY;

pub(crate) async fn remove_album_directory(
    root: &Path,
    relative_path: &str,
) -> std::io::Result<bool> {
    // Catalog locations have exactly two components. Accepting an artist directory
    // or normalizing dot components would widen the scope of a destructive request.
    let components: Vec<_> = relative_path.split('/').collect();
    if components.len() != 2
        || components.iter().any(|part| {
            part.is_empty()
                || matches!(*part, "." | "..")
                || *part == STAGING_DIRECTORY
                || part.contains(['\\', '\0', ':'])
        })
    {
        return Err(std::io::Error::other("Unsafe recorded album location"));
    }
    if root.as_os_str().is_empty() {
        return Err(std::io::Error::other("Unsafe configured music directory"));
    }
    match tokio::fs::remove_dir_all(root.join(relative_path)).await {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
