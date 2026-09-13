use std::{collections::HashMap, num::ParseIntError};

use super::tidal::TidalAlbum;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct AlbumIdentity {
    pub(crate) normalized_title: String,
    pub(crate) release_type: String,
}

impl AlbumIdentity {
    pub(crate) fn new(title: &str, release_type: &str) -> Self {
        Self {
            normalized_title: title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
            release_type: release_type.to_owned(),
        }
    }
}

impl From<&TidalAlbum> for AlbumIdentity {
    fn from(album: &TidalAlbum) -> Self {
        Self::new(&album.title, &album.r#type)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid numeric Tidal album ID {album_id}: {source}")]
pub(crate) struct InvalidAlbumId {
    album_id: String,
    source: ParseIntError,
}

fn candidate_rank(album: &TidalAlbum) -> Result<(u8, u8, u64), InvalidAlbumId> {
    let explicitness_rank = match album.explicit {
        Some(true) => 2,
        None => 1,
        Some(false) => 0,
    };
    let quality_rank = album
        .media_tags
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|tag| match tag.as_str() {
            "HIRES_LOSSLESS" => 3,
            "LOSSLESS" => 2,
            "DOLBY_ATMOS" => 1,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    let numeric_album_id = album.id.parse::<u64>().map_err(|source| InvalidAlbumId {
        album_id: album.id.clone(),
        source,
    })?;
    Ok((explicitness_rank, quality_rank, numeric_album_id))
}

pub(crate) fn select_candidates(
    candidate_albums: impl IntoIterator<Item = TidalAlbum>,
) -> Result<Vec<TidalAlbum>, InvalidAlbumId> {
    let mut selected_by_identity = HashMap::new();
    let mut selected_albums: Vec<TidalAlbum> = Vec::new();
    for candidate in candidate_albums {
        let identity = AlbumIdentity::from(&candidate);
        let candidate_rank = candidate_rank(&candidate)?;
        if let Some((selected_index, selected_rank)) = selected_by_identity.get_mut(&identity) {
            if candidate_rank > *selected_rank {
                selected_albums[*selected_index] = candidate;
                *selected_rank = candidate_rank;
            }
        } else {
            selected_by_identity.insert(identity, (selected_albums.len(), candidate_rank));
            selected_albums.push(candidate);
        }
    }
    Ok(selected_albums)
}
