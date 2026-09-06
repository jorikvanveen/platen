import { render } from "svelte/server";
import { describe, expect, it } from "vitest";
import type { CatalogScan } from "$lib/dto/CatalogScan";
import ImportPage from "./+page.svelte";

const summary: CatalogScan["summary"] = {
	album_directories_found: 21,
	candidates_processed: 18,
	candidates_total: 18,
	albums_imported: 4,
	locations_attached: 2,
	locations_changed: 1,
	unchanged_locations: 3,
	locations_cleared: 5,
	unmatched_candidates: 6,
	ambiguous_matches: 7,
	duplicate_locations: 8,
	skipped_directories: 9,
	filesystem_errors: 10,
	failures: 11,
};

function renderScan(phase: CatalogScan["phase"], counts = summary) {
	return render(ImportPage, {
		props: { data: { scan: { phase, summary: counts, failure_reason: null } }, params: {} },
	}).body;
}

describe("Import page", () => {
	it.each([
		["scanning", "Scanning the filesystem"],
		["matching", "Matching candidates against Tidal"],
	] as const)("displays the %s phase and prevents another scan", (phase, label) => {
		const html = renderScan(phase, { ...summary, candidates_processed: 12 });

		expect(html).toMatch(new RegExp(`<h2[^>]*>${label}</h2>`));
		expect(html).toMatch(/<strong[^>]*>12 \/ 18<\/strong>/);
		expect(html).toMatch(/<button[^>]*disabled[^>]*>Scan running<\/button>/);
	});

	it("displays every final aggregate count without ambiguity-resolution controls", () => {
		const html = renderScan("completed");

		expect(html).toMatch(/<h2[^>]*>Completed<\/h2>/);
		for (const [label, value] of [
			["Album directories", 21],
			["Candidates processed", "18 / 18"],
			["Albums imported", 4],
			["Locations attached", 2],
			["Locations changed", 1],
			["Locations unchanged", 3],
			["Locations cleared", 5],
			["Unmatched candidates", 6],
			["Ambiguous matches", 7],
			["Duplicate locations skipped", 8],
			["Skipped directories", 9],
			["Filesystem errors", 10],
			["Tidal or database failures", 11],
		]) {
			expect(html).toMatch(new RegExp(`<span[^>]*>${label}</span>\\s*<strong[^>]*>${value}</strong>`));
		}
		expect(html.match(/<button\b/g)).toHaveLength(1);
		expect(html).toMatch(/<button>Start scan<\/button>/);
		expect(html).not.toMatch(/<(?:select|input)\b/);
	});
});
