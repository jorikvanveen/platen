import type { TidalAlbum } from "$lib/dto/TidalAlbum";

export type AlbumGroup = {
  label: string;
  albums: TidalAlbum[];
};

const canonical = [
  { key: "ALBUM", label: "Albums" },
  { key: "EP", label: "EPs" },
  { key: "SINGLE", label: "Singles" },
];

export function sortByReleaseDate(albums: TidalAlbum[]): TidalAlbum[] {
  return albums
    .map((album, index) => ({ album, index }))
    .sort((a, b) => {
      if (a.album.release_date === null && b.album.release_date === null) return a.index - b.index;
      if (a.album.release_date === null) return 1;
      if (b.album.release_date === null) return -1;
      const cmp = b.album.release_date.localeCompare(a.album.release_date);
      return cmp !== 0 ? cmp : a.index - b.index;
    })
    .map(({ album }) => album);
}

export function groupAlbums(albums: TidalAlbum[]): AlbumGroup[] {
  const buckets: Record<string, TidalAlbum[]> = {};
  const unknown: TidalAlbum[] = [];

  for (const album of albums) {
    const type = (album.album_type || "").toUpperCase();
    if (type === "ALBUM" || type === "EP" || type === "SINGLE") {
      (buckets[type] ??= []).push(album);
    } else {
      unknown.push(album);
    }
  }

  const groups: AlbumGroup[] = [];
  for (const { key, label } of canonical) {
    const list = buckets[key];
    if (list && list.length > 0) {
      groups.push({ label, albums: sortByReleaseDate(list) });
    }
  }
  if (unknown.length > 0) {
    groups.push({ label: "Unknown", albums: sortByReleaseDate(unknown) });
  }
  return groups;
}