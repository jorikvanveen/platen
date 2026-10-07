Read and follow `CODING_STANDARDS.md` before making code changes.
Put specs, scratch files, and other session artifacts in `./.agents/`.
Put architecture decision records in `./docs/adr/`.

Generated files (SeaORM entities, TS bindings, etc) must be regenerated with the
scripts in platen-backend/scripts/, never edited by hand:
- platen-backend/scripts/*.sh
