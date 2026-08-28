# 0003: Sanitize filesystem-hostile characters in catalog-derived directory names

**What to build:** A brief goal statement, deliberately not a spec. Catalog
titles can contain characters that are hostile to filesystem paths (for
example the path separator in `Speakerboxxx/The Love Below`), and directory
names derived from catalog metadata must be safe to create on disk. The
library layout is computed from catalog metadata per ADR 0003, so
sanitization belongs at the point where catalog names become directory
names.

This ticket is intentionally left coarse. After the new placement logic has
been implemented and lessons taken from that work, run a grilling session on
sanitization and expand this ticket with the resulting decisions before
implementing it.

**Blocked by:** 0002 (Place album and EP downloads in catalog-derived
directories).

**Status:** ready-for-agent

- [ ] Grilling session on sanitization held after 0002 lands, incorporating lessons from its implementation
- [ ] Ticket expanded with the resulting decisions
- [ ] Catalog-derived directory names are safe on disk for hostile titles
