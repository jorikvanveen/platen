import { describe, expect, it, vi } from "vitest";
import type { CatalogScan } from "$lib/dto/CatalogScan";
import {
	CatalogScanConflictError,
	CatalogScanRequestError,
	getCatalogScan,
	isCatalogScanActive,

	startCatalogScan,
} from "./catalogScan";
import { pollCatalogScan } from "./importController";

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
		expect(fetcher).toHaveBeenCalledWith("/api/catalog/scan", { signal: undefined });
	});

	it("starts a scan with POST and decodes its status", async () => {
		const active = scan("scanning");
		const fetcher = vi.fn().mockResolvedValue(response(active, 202));

		await expect(startCatalogScan(fetcher)).resolves.toEqual(active);
		expect(fetcher).toHaveBeenCalledWith("/api/catalog/scan", { method: "POST", signal: undefined });
	});

	it("returns the active status with a conflict error", async () => {
		const active = scan("matching");
		const fetcher = vi.fn().mockResolvedValue(response(active, 409));

		const error = await startCatalogScan(fetcher).catch((caught) => caught);
		expect(error).toBeInstanceOf(CatalogScanConflictError);
		expect(error.activeScan).toEqual(active);
	});

	it("polls until a completed status and then stops", async () => {
		const updates: Array<CatalogScan | null> = [];
		const fetcher = vi
			.fn()
			.mockResolvedValueOnce(response(scan("matching")))
			.mockResolvedValueOnce(response(scan("completed")));

		const result = await pollCatalogScan(fetcher, (status) => updates.push(status), {
			initial: scan("scanning"),
			sleep: async () => {},
		});

		expect(result?.phase).toBe("completed");
		expect(updates.map((status) => status?.phase)).toEqual(["scanning", "matching", "completed"]);
		expect(fetcher).toHaveBeenCalledTimes(2);
	});

	it("resumes matching, publishes changing counters, and retains final outcomes", async () => {
		const initial = {
			...scan("matching"),
			summary: { ...emptySummary, candidates_total: 8, candidates_processed: 2 },
		};
		const matching = { ...initial, summary: { ...initial.summary, candidates_processed: 6 } };
		const completed: CatalogScan = {
			phase: "completed",
			summary: {
				...matching.summary,
				candidates_processed: 8,
				albums_imported: 3,
				unmatched_candidates: 2,
				ambiguous_matches: 1,
				duplicate_locations: 2,
			},
			failure_reason: null,
		};
		const fetcher = vi.fn()
			.mockResolvedValueOnce(response(matching))
			.mockResolvedValueOnce(response(completed));
		const onUpdate = vi.fn();
		const sleep = vi.fn().mockResolvedValue(undefined);

		await expect(pollCatalogScan(fetcher, onUpdate, { initial, sleep })).resolves.toEqual(completed);
		expect(onUpdate.mock.calls.map(([status]) => status)).toEqual([initial, matching, completed]);
		expect(fetcher).toHaveBeenCalledTimes(2);
		expect(sleep.mock.calls).toEqual([[1000], [1000]]);
	});

	it.each(["completed", "failed"] as const)("does not poll an initial %s status", async (phase) => {
		const initial = scan(phase);
		const fetcher = vi.fn();
		const onUpdate = vi.fn();
		const sleep = vi.fn();

		await expect(pollCatalogScan(fetcher, onUpdate, { initial, sleep })).resolves.toEqual(initial);
		expect(onUpdate).toHaveBeenCalledExactlyOnceWith(initial);
		expect(fetcher).not.toHaveBeenCalled();
		expect(sleep).not.toHaveBeenCalled();
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
