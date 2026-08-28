# Library layout is computed from catalog metadata, never from downloader archives

Antra's ZIP archives arrive in folders whose names we cannot control and
cannot trust to match the catalog; on a collaboration album the folder is
named after artists other than the catalog artist. Jellyfin reads those
folder names as artist names, spawning phantom artists in the library. We
decided that platen always computes the destination directory from its own
catalog (artist name, album title, release year) and treats the archive's
internal structure as disposable; the archive is extracted to a temporary
location and only its files are placed. The alternative, trusting or
repairing the archive's layout, would keep the library at the mercy of a
third party's tagging quirks.
