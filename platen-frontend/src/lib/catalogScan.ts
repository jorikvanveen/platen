import { API_URL } from "$lib/constants";
import type { CatalogScan } from "$lib/dto/CatalogScan";

export type Fetcher = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export class CatalogScanRequestError extends Error {
	constructor(
		message: string,
		public readonly status: number,
	) {
		super(message);
	}
}

export class CatalogScanConflictError extends CatalogScanRequestError {
	constructor(public readonly activeScan: CatalogScan) {
		super("A Music directory scan is already running.", 409);
	}
}

export function isCatalogScanActive(scan: CatalogScan | null): boolean {
	return scan?.phase === "scanning" || scan?.phase === "matching";
}

export async function getCatalogScan(fetcher: Fetcher): Promise<CatalogScan | null> {
	const response = await fetcher(`${API_URL}/catalog/scan`);
	if (!response.ok) {
		throw new CatalogScanRequestError("Could not load the Music directory scan.", response.status);
	}
	const body: unknown = await response.json();
	return body === null ? null : decodeCatalogScan(body);
}

export async function startCatalogScan(fetcher: Fetcher): Promise<CatalogScan> {
	const response = await fetcher(`${API_URL}/catalog/scan`, { method: "POST" });
	if (response.status === 409) {
		throw new CatalogScanConflictError(decodeCatalogScan(await response.json()));
	}
	if (response.status !== 202) {
		throw new CatalogScanRequestError("Could not start the Music directory scan.", response.status);
	}
	return decodeCatalogScan(await response.json());
}

export async function pollCatalogScan(
	fetcher: Fetcher,
	onUpdate: (scan: CatalogScan) => void,
	options: {
		initial?: CatalogScan;
		intervalMs?: number;
		sleep?: (milliseconds: number) => Promise<void>;
	} = {},
): Promise<CatalogScan | null> {
	const sleep = options.sleep ?? ((milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)));
	let scan = options.initial ?? (await getCatalogScan(fetcher));
	if (scan) onUpdate(scan);

	while (isCatalogScanActive(scan)) {
		await sleep(options.intervalMs ?? 1000);
		scan = await getCatalogScan(fetcher);
		if (scan) onUpdate(scan);
	}
	return scan;
}

function decodeCatalogScan(value: unknown): CatalogScan {
	if (!isRecord(value) || !isPhase(value.phase) || !isRecord(value.summary)) {
		throw new CatalogScanRequestError("The server returned an invalid Music scan status.", 502);
	}
	const countFields = [
		"album_directories_found",
		"candidates_processed",
		"candidates_total",
		"albums_imported",
		"locations_attached",
		"locations_changed",
		"unchanged_locations",
		"locations_cleared",
		"unmatched_candidates",
		"ambiguous_matches",
		"duplicate_locations",
		"skipped_directories",
		"failures",
		"filesystem_errors",
	];
	for (const field of countFields) {
		const count = value.summary[field];
		if (typeof count !== "number" || !Number.isInteger(count) || count < 0) {
			throw new CatalogScanRequestError("The server returned an invalid Music scan summary.", 502);
		}
	}
	if (value.failure_reason !== null && typeof value.failure_reason !== "string") {
		throw new CatalogScanRequestError("The server returned an invalid Music scan failure.", 502);
	}
	return value as CatalogScan;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function isPhase(value: unknown): boolean {
	return value === "scanning" || value === "matching" || value === "completed" || value === "failed";
}
