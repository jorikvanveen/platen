import { describe, expect, it } from "vitest";
import { groupAlbums, sortByReleaseDate } from "$lib/albumGroups";
import type { TidalAlbum } from "$lib/dto/TidalAlbum";

function album(id: string, album_type: string, release_date: string | null): TidalAlbum {
  return { id, title: id, album_type, release_date, popularity: 0 };
}

describe("sortByReleaseDate", () => {
  it("sorts newest first", () => {
    const albums = [
      album("old", "ALBUM", "2020-01-01"),
      album("new", "ALBUM", "2024-05-05"),
      album("mid", "ALBUM", "2022-03-03"),
    ];
    expect(sortByReleaseDate(albums).map((a) => a.id)).toEqual(["new", "mid", "old"]);
  });

  it("sorts null release dates last", () => {
    const albums = [
      album("null1", "ALBUM", null),
      album("dated", "ALBUM", "2023-01-01"),
      album("null2", "ALBUM", null),
    ];
    expect(sortByReleaseDate(albums).map((a) => a.id)).toEqual(["dated", "null1", "null2"]);
  });

  it("preserves input order on ties", () => {
    const albums = [
      album("first", "ALBUM", "2021-01-01"),
      album("second", "ALBUM", "2021-01-01"),
      album("third", "ALBUM", "2021-01-01"),
    ];
    expect(sortByReleaseDate(albums).map((a) => a.id)).toEqual(["first", "second", "third"]);
  });

  it("does not mutate the input array", () => {
    const albums = [
      album("a", "ALBUM", "2020-01-01"),
      album("b", "ALBUM", "2024-05-05"),
    ];
    const input = [...albums];
    sortByReleaseDate(albums);
    expect(albums).toEqual(input);
  });
});

describe("groupAlbums", () => {
  it("groups canonical types in order Albums, EPs, Singles, Unknown", () => {
    const albums = [
      album("s1", "SINGLE", "2024-01-01"),
      album("a1", "ALBUM", "2020-01-01"),
      album("u1", "OTHER", "2021-01-01"),
      album("e1", "EP", "2022-01-01"),
    ];
    const groups = groupAlbums(albums);
    expect(groups.map((g) => g.label)).toEqual(["Albums", "EPs", "Singles", "Unknown"]);
    expect(groups[0].albums.map((a) => a.id)).toEqual(["a1"]);
    expect(groups[1].albums.map((a) => a.id)).toEqual(["e1"]);
    expect(groups[2].albums.map((a) => a.id)).toEqual(["s1"]);
    expect(groups[3].albums.map((a) => a.id)).toEqual(["u1"]);
  });

  it("matches album_type case-insensitively", () => {
    const albums = [
      album("lower", "album", "2020-01-01"),
      album("mixed", "Ep", "2021-01-01"),
      album("upper", "SINGLE", "2022-01-01"),
    ];
    const groups = groupAlbums(albums);
    expect(groups.map((g) => g.label)).toEqual(["Albums", "EPs", "Singles"]);
    expect(groups[0].albums.map((a) => a.id)).toEqual(["lower"]);
    expect(groups[1].albums.map((a) => a.id)).toEqual(["mixed"]);
    expect(groups[2].albums.map((a) => a.id)).toEqual(["upper"]);
  });

  it("collapses empty and unrecognized types into Unknown", () => {
    const albums = [
      album("empty", "", "2020-01-01"),
      album("weird", "LIVE", "2021-01-01"),
      album("other", "COMPILATION", "2022-01-01"),
    ];
    const groups = groupAlbums(albums);
    expect(groups).toHaveLength(1);
    expect(groups[0].label).toBe("Unknown");
    expect(groups[0].albums.map((a) => a.id)).toEqual(["other", "weird", "empty"]);
  });

  it("omits canonical groups with zero albums", () => {
    const albums = [album("s1", "SINGLE", "2024-01-01")];
    const groups = groupAlbums(albums);
    expect(groups.map((g) => g.label)).toEqual(["Singles"]);
  });

  it("omits Unknown when there are no non-canonical albums", () => {
    const albums = [album("a1", "ALBUM", "2020-01-01")];
    const groups = groupAlbums(albums);
    expect(groups.map((g) => g.label)).toEqual(["Albums"]);
  });

  it("returns no groups for an empty input", () => {
    expect(groupAlbums([])).toEqual([]);
  });

  it("sorts within each group newest first", () => {
    const albums = [
      album("old", "ALBUM", "2020-01-01"),
      album("new", "ALBUM", "2024-05-05"),
      album("undated", "ALBUM", null),
    ];
    const groups = groupAlbums(albums);
    expect(groups[0].albums.map((a) => a.id)).toEqual(["new", "old", "undated"]);
  });
});