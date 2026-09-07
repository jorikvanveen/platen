import { render } from "svelte/server";
import { describe, expect, it, vi } from "vitest";
import DownloadsPage from "../routes/downloads/+page.svelte";
import { load } from "../routes/downloads/+page";
import type { DownloadJob } from "$lib/dto/DownloadJob";
import type { Downloads } from "$lib/dto/Downloads";

const job: DownloadJob = {
	id: "job-1", album_id: "album-1", release_name: "Example Album",
	explicit: null, available_quality: null, status: "queued",
	enqueued_at: "2026-09-06T12:00:00Z", started_at: null,
	finished_at: null, failure_reason: null,
};

function renderDownloads(downloads: Downloads) {
	return render(DownloadsPage, { props: { data: { downloads }, params: {} } }).body;
}

function tableRows(html: string) {
	return [...html.matchAll(/<tbody[^>]*>([\s\S]*?)<\/tbody>/g)].flatMap(table =>
		[...table[1].matchAll(/<tr[^>]*>([\s\S]*?)<\/tr>/g)].map(row =>
			[...row[1].matchAll(/<td[^>]*>([\s\S]*?)<\/td>/g)].map(cell =>
				cell[1].replace(/<span class="mobile-label[^>]*>.*?<\/span>/g, "")
					.replace(/<[^>]*>/g, "").trim(),
			),
		),
	);
}

describe("Downloads metadata", () => {
	it.each([
		{ explicit: true, available_quality: "LOSSLESS", label: "Explicit" },
		{ explicit: false, available_quality: "HIRES_LOSSLESS", label: "Not explicit" },
		{ explicit: null, available_quality: "DOLBY_ATMOS", label: "Unknown" },
		{ explicit: true, available_quality: "LOSSLESS + DOLBY_ATMOS", label: "Explicit" },
		{ explicit: false, available_quality: "HIRES_LOSSLESS + DOLBY_ATMOS", label: "Not explicit" },
		{ explicit: true, available_quality: null, label: "Explicit" },
		{ explicit: false, available_quality: null, label: "Not explicit" },
		{ explicit: null, available_quality: null, label: "Unknown" },
	])("shows metadata in active downloads and history: %j", async ({ label, ...metadata }) => {
		const downloads: Downloads = {
			active: [{ ...job, ...metadata }],
			history: [{ ...job, ...metadata, id: "job-2", status: "succeeded" }],
		};
		const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValueOnce(Response.json(downloads));
		expect(await load({ fetch } as unknown as Parameters<typeof load>[0])).toEqual({ downloads });
		const html = renderDownloads(downloads);
		const rows = tableRows(html);
		expect(rows).toHaveLength(2);
		for (const row of rows) {
			expect(row.slice(0, 3)).toEqual(["Example Album", label, metadata.available_quality ?? "Unknown"]);
		}
		expect(html).toContain("Active downloads");
		expect(html).toContain("History");
		expect(html).toContain("Available quality");
		expect(html).toContain("availability on Tidal, not the downloaded audio format");
		expect(html).not.toContain("Clean");
	});

	it("preserves statuses, failure information, and cancellation only for queued jobs", () => {
		const html = renderDownloads({
			active: [job, { ...job, id: "running", status: "running" }],
			history: [
				{ ...job, id: "failed", status: "failed", failure_reason: "Album download failed." },
				{ ...job, id: "cancelled", status: "cancelled" },
				{ ...job, id: "succeeded", status: "succeeded" },
			],
		});
		const rows = tableRows(html);
		expect(rows.map(row => row[3])).toEqual(["queued Cancel", "running", "failed", "cancelled", "succeeded"]);
		expect(rows[2][4]).toBe("Album download failed.");
		expect(html.match(/<button\b/g)).toHaveLength(1);
	});

	it("falls back to the Tidal ID and Unknown for deleted Albums in both lists", () => {
		const deleted = { ...job, release_name: null };
		const rows = tableRows(renderDownloads({
			active: [deleted],
			history: [{ ...deleted, id: "old-job", status: "cancelled" }],
		}));
		for (const row of rows) {
			expect(row.slice(0, 3)).toEqual(["album-1", "Unknown", "Unknown"]);
		}
	});

	it("preserves empty states", () => {
		const html = renderDownloads({ active: [], history: [] });
		expect(html).toContain("No active downloads.");
		expect(html).toContain("No download history.");
		expect(html).not.toContain("<table");
	});
});
