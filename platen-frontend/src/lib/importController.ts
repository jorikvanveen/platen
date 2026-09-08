import { writable } from "svelte/store";
import {
	CatalogScanConflictError,
	getCatalogScan,
	isCatalogScanActive,
	startCatalogScan,
	type Fetcher,
} from "$lib/catalogScan";
import type { CatalogScan } from "$lib/dto/CatalogScan";

export interface ImportState {
	scan: CatalogScan | null;
	starting: boolean;
	following: boolean;
	error: string | null;
}

interface PollOptions {
	intervalMs?: number;
	sleep?: (milliseconds: number) => Promise<void>;
}

export async function pollCatalogScan(
	fetcher: Fetcher,
	onUpdate: (scan: CatalogScan | null) => void,
	options: PollOptions & { initial?: CatalogScan; signal?: AbortSignal } = {},
): Promise<CatalogScan | null> {
	const { signal } = options;
	signal?.throwIfAborted();
	let scan = options.initial ?? (await getCatalogScan(fetcher, signal));
	signal?.throwIfAborted();
	onUpdate(scan);
	while (isCatalogScanActive(scan)) {
		if (options.sleep) await options.sleep(options.intervalMs ?? 1000);
		else await wait(options.intervalMs ?? 1000, signal);
		signal?.throwIfAborted();
		scan = await getCatalogScan(fetcher, signal);
		signal?.throwIfAborted();
		onUpdate(scan);
	}
	return scan;
}

function wait(milliseconds: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		const abort = () => {
			clearTimeout(timer);
			reject(signal?.reason);
		};
		const timer = setTimeout(() => {
			signal?.removeEventListener("abort", abort);
			resolve();
		}, milliseconds);
		signal?.addEventListener("abort", abort, { once: true });
		if (signal?.aborted) abort();
	});
}

export function createImportController(
	fetcher: Fetcher,
	initial: CatalogScan | null,
	options: PollOptions = {},
) {
	let state: ImportState = { scan: initial, starting: false, following: false, error: null };
	const store = writable(state);
	let operation: AbortController | null = null;
	let disposed = false;

	function update(changes: Partial<ImportState>) {
		if (disposed) return;
		state = { ...state, ...changes };
		store.set(state);
	}

	async function run(start: boolean) {
		if (disposed || operation) return;
		const controller = new AbortController();
		operation = controller;
		const { signal } = controller;
		update({ starting: start, following: !start, error: null });
		try {
			let current = state.scan;
			if (start) {
				try {
					current = await startCatalogScan(fetcher, signal);
				} catch (error) {
					if (!(error instanceof CatalogScanConflictError)) throw error;
					current = error.activeScan;
				}
				signal.throwIfAborted();
				update({ scan: current, starting: false });
			}
			if (current && isCatalogScanActive(current)) {
				update({ following: true });
				await pollCatalogScan(fetcher, (scan) => update({ scan }), { ...options, initial: current, signal });
			}
		} catch (error) {
			if (!signal.aborted) {
				update({ error: error instanceof Error ? error.message : "Could not load the Music directory scan." });
			}
		} finally {
			operation = null;
			update({ starting: false, following: false });
		}
	}

	return {
		subscribe: store.subscribe,
		start: () => run(!isCatalogScanActive(state.scan)),
		resume: () => isCatalogScanActive(state.scan) ? run(false) : Promise.resolve(),
		dispose() {
			disposed = true;
			operation?.abort();
		},
	};
}
