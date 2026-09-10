#!/usr/bin/env sh
set -eu

cd "$(dirname "$0")/../../platen-frontend"

# Pin the CLI and neutral Nova preset to keep the setup consistent without interactive prompts.
npx --yes shadcn-svelte@1.6.1 init \
	--preset b0 \
	--skip-preflight \
	--base-color neutral \
	--css src/app.css \
	--components-alias '$lib/components' \
	--lib-alias '$lib' \
	--utils-alias '$lib/utils' \
	--hooks-alias '$lib/hooks' \
	--ui-alias '$lib/components/ui' \
	--reinstall

npx --yes shadcn-svelte@1.6.1 add card avatar badge --yes --overwrite --skip-preflight

# Rename upstream's shorthand here so regeneration preserves our descriptive helper name.
node --input-type=module <<'NODE'
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const uiDirectory = 'src/lib/components/ui';
const generatedFiles = [
	'src/lib/utils.ts',
	...readdirSync(uiDirectory, { recursive: true })
		.filter((relativePath) => /\.(svelte|ts)$/.test(relativePath))
		.map((relativePath) => join(uiDirectory, relativePath))
];

for (const filePath of generatedFiles) {
	const source = readFileSync(filePath, 'utf8');
	writeFileSync(filePath, source.replace(/\bcn\b/g, 'mergeClasses'));
}
NODE
