import { get } from "svelte/store";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CatalogScan } from "$lib/dto/CatalogScan";
import { createImportController, pollCatalogScan } from "./importController";

function scan(phase: CatalogScan["phase"]): CatalogScan {
	return {
		phase, failure_reason: null,
		summary: {
			album_directories_found: 1, candidates_processed: 0, candidates_total: 1,
			albums_imported: 0, locations_attached: 0, locations_changed: 0,
			unchanged_locations: 0, locations_cleared: 0, unmatched_candidates: 0,
			ambiguous_matches: 0, duplicate_locations: 0, skipped_directories: 0,
			failures: 0, filesystem_errors: 0,
		},
	};
}

function response(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((complete) => { resolve = complete; });
	return { promise, resolve };
}

afterEach(() => vi.useRealTimers());

describe("Import controller", () => {
	it("blocks duplicate starts before the POST returns", async () => {
		const pending = deferred<Response>();
		const fetcher = vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValue(response(scan("completed")));
		const controller = createImportController(fetcher, null, { sleep: async () => {} });
		const start = controller.start();
		expect(get(controller).starting).toBe(true);
		await controller.start();
		await controller.resume();
		expect(fetcher).toHaveBeenCalledTimes(1);
		pending.resolve(response(scan("scanning"), 202));
		await start;
		expect(get(controller)).toMatchObject({ starting: false, following: false, error: null, scan: { phase: "completed" } });
		expect(fetcher.mock.calls.map(([, init]) => init.method ?? "GET")).toEqual(["POST", "GET"]);
	});

	it("follows an existing scan returned by a conflict", async () => {
		const fetcher = vi.fn().mockResolvedValueOnce(response(scan("matching"), 409)).mockResolvedValue(response(scan("completed")));
		const controller = createImportController(fetcher, null, { sleep: async () => {} });
		await controller.start();
		expect(get(controller).scan?.phase).toBe("completed");
		expect(get(controller).error).toBeNull();
		expect(fetcher).toHaveBeenCalledTimes(2);
	});

	it("resumes after a polling failure without starting another scan", async () => {
		const fetcher = vi.fn().mockRejectedValueOnce(new Error("Offline")).mockResolvedValue(response(scan("completed")));
		const initial = scan("matching");
		const controller = createImportController(fetcher, initial, { sleep: async () => {} });
		await controller.resume();
		expect(get(controller)).toEqual({ scan: initial, starting: false, following: false, error: "Offline" });
		await controller.resume();
		expect(get(controller).scan?.phase).toBe("completed");
		expect(get(controller).error).toBeNull();
		expect(fetcher.mock.calls.every(([, init]) => !init.method)).toBe(true);
	});

	it("retains the last result when a new start fails and allows retry", async () => {
		const initial = scan("completed");
		const fetcher = vi.fn().mockResolvedValueOnce(response({}, 500)).mockResolvedValue(response(scan("failed"), 202));
		const controller = createImportController(fetcher, initial);
		await controller.start();
		expect(get(controller).scan).toEqual(initial);
		expect(get(controller).starting).toBe(false);
		expect(get(controller).error).toContain("Could not start");
		await controller.start();
		expect(get(controller).scan?.phase).toBe("failed");
		expect(get(controller).error).toBeNull();
	});

	it("clears stale active state when the server has restarted", async () => {
		const fetcher = vi.fn().mockResolvedValue(response(null));
		const controller = createImportController(fetcher, scan("matching"), { sleep: async () => {} });
		await controller.resume();
		expect(get(controller)).toEqual({ scan: null, starting: false, following: false, error: null });
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	it("aborts a pending polling delay when disposed", async () => {
		vi.useFakeTimers();
		const fetcher = vi.fn();
		const controller = createImportController(fetcher, scan("scanning"));
		const following = controller.resume();
		expect(vi.getTimerCount()).toBe(1);
		controller.dispose();
		await following;
		expect(vi.getTimerCount()).toBe(0);
		await controller.start();
		expect(fetcher).not.toHaveBeenCalled();
	});

	it.each(["start", "resume"] as const)("aborts %s and ignores a late response after disposal", async (action) => {
		const pending = deferred<Response>();
		const fetcher = vi.fn().mockReturnValue(pending.promise);
		const controller = createImportController(fetcher, action === "resume" ? scan("matching") : null, { sleep: async () => {} });
		const updates = vi.fn();
		controller.subscribe(updates);
		const running = controller[action]();
		await Promise.resolve();
		const signal: AbortSignal = fetcher.mock.calls[0][1].signal;
		controller.dispose();
		const count = updates.mock.calls.length;
		expect(signal.aborted).toBe(true);
		pending.resolve(response(scan("completed"), action === "start" ? 202 : 200));
		await running;
		expect(updates).toHaveBeenCalledTimes(count);
		expect(fetcher).toHaveBeenCalledTimes(1);
	});

	it("does not let controllers from separate pages share request state", async () => {
		const fetcher = vi.fn().mockResolvedValue(response(scan("completed"), 202));
		const first = createImportController(fetcher, null);
		const second = createImportController(fetcher, null);
		first.dispose();
		await second.start();
		expect(get(first).scan).toBeNull();
		expect(get(second).scan?.phase).toBe("completed");
	});
});

describe("abortable scan polling", () => {
	it("does not fetch or publish with an already aborted signal", async () => {
		const controller = new AbortController();
		controller.abort();
		const fetcher = vi.fn();
		const onUpdate = vi.fn();
		await expect(pollCatalogScan(fetcher, onUpdate, { signal: controller.signal })).rejects.toMatchObject({ name: "AbortError" });
		expect(fetcher).not.toHaveBeenCalled();
		expect(onUpdate).not.toHaveBeenCalled();
	});
});
