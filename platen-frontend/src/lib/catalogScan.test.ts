import { describe, expect, it, vi } from "vitest";
import type { CatalogScan } from "$lib/dto/CatalogScan";
import {
	CatalogScanConflictError,
	CatalogScanRequestError,
	getCatalogScan,
	isCatalogScanActive,
	pollCatalogScan,
	startCatalogScan,
} from "./catalogScan";

const emptySummary = {
	album_directories_found: 0,
	candidates_processed: 0,
	candidates_total: 0,
	albums_imported: 0,
	locations_attached: 0,
	locations_changed: 0,
	unchanged_locations: 0,
	locations_cleared: 0,
	unmatched_candidates: 0,
	ambiguous_matches: 0,
	duplicate_locations: 0,
	skipped_directories: 0,
	failures: 0,
	filesystem_errors: 0,
};

function scan(phase: CatalogScan["phase"]): CatalogScan {
	return { phase, summary: emptySummary, failure_reason: null };
}

function response(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { "content-type": "application/json" },
	});
}

describe("catalog scan requests", () => {
	it("identifies the phases that require polling", () => {
		expect(isCatalogScanActive(null)).toBe(false);
		expect(isCatalogScanActive(scan("scanning"))).toBe(true);
		expect(isCatalogScanActive(scan("matching"))).toBe(true);
		expect(isCatalogScanActive(scan("completed"))).toBe(false);
		expect(isCatalogScanActive(scan("failed"))).toBe(false);
	});

	it("loads no status before the first scan", async () => {
		const fetcher = vi.fn().mockResolvedValue(response(null));

		await expect(getCatalogScan(fetcher)).resolves.toBeNull();
		expect(fetcher).toHaveBeenCalledWith("/api/catalog/scan");
	});

	it("starts a scan with POST and decodes its status", async () => {
		const active = scan("scanning");
		const fetcher = vi.fn().mockResolvedValue(response(active, 202));

		await expect(startCatalogScan(fetcher)).resolves.toEqual(active);
		expect(fetcher).toHaveBeenCalledWith("/api/catalog/scan", { method: "POST" });
	});

	it("returns the active status with a conflict error", async () => {
		const active = scan("matching");
		const fetcher = vi.fn().mockResolvedValue(response(active, 409));

		const error = await startCatalogScan(fetcher).catch((caught) => caught);
		expect(error).toBeInstanceOf(CatalogScanConflictError);
		expect(error.activeScan).toEqual(active);
	});

	it("polls until a completed status and then stops", async () => {
		const updates: CatalogScan[] = [];
		const fetcher = vi
			.fn()
			.mockResolvedValueOnce(response(scan("matching")))
			.mockResolvedValueOnce(response(scan("completed")));

		const result = await pollCatalogScan(fetcher, (status) => updates.push(status), {
			initial: scan("scanning"),
			sleep: async () => {},
		});

		expect(result?.phase).toBe("completed");
		expect(updates.map(({ phase }) => phase)).toEqual(["scanning", "matching", "completed"]);
		expect(fetcher).toHaveBeenCalledTimes(2);
	});

	it("stops polling when the scan fails", async () => {
		const failed = { ...scan("failed"), failure_reason: "Could not scan the Music directory." };
		const fetcher = vi.fn().mockResolvedValue(response(failed));

		await expect(
			pollCatalogScan(fetcher, () => {}, { initial: scan("scanning"), sleep: async () => {} }),
		).resolves.toEqual(failed);
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	it("rejects HTTP errors and malformed summaries", async () => {
		await expect(getCatalogScan(vi.fn().mockResolvedValue(response({}, 500)))).rejects.toMatchObject({
			status: 500,
		});
		await expect(
			getCatalogScan(
				vi.fn().mockResolvedValue(
					response({ ...scan("completed"), summary: { ...emptySummary, failures: "one" } }),
				),
			),
		).rejects.toBeInstanceOf(CatalogScanRequestError);
	});
});
