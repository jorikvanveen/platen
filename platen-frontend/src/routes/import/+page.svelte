<script lang="ts">
	import { onMount } from "svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import {
		CatalogScanConflictError,
		isCatalogScanActive,
		pollCatalogScan,
		startCatalogScan,
	} from "$lib/catalogScan";
	import type { CatalogScan } from "$lib/dto/CatalogScan";
	import type { PageProps } from "./$types";

	let { data }: PageProps = $props();
	// svelte-ignore state_referenced_locally -- data.scan only seeds local state; polling owns later updates.
	let scan = $state<CatalogScan | null>(data.scan);
	let requestError = $state<string | null>(null);
	let polling = false;

	const active = $derived(isCatalogScanActive(scan));
	const phaseLabel = $derived(
		scan?.phase === "scanning"
			? "Scanning the filesystem"
			: scan?.phase === "matching"
				? "Preparing Catalog candidates"
				: scan?.phase === "completed"
					? "Completed"
					: scan?.phase === "failed"
						? "Failed"
						: "Not run yet",
	);

	async function follow(current: CatalogScan) {
		if (polling) return;
		polling = true;
		try {
			await pollCatalogScan(fetch, (status) => (scan = status), { initial: current });
		} catch (error) {
			requestError = error instanceof Error ? error.message : "Could not load scan progress.";
		} finally {
			polling = false;
		}
	}

	async function start() {
		requestError = null;
		try {
			scan = await startCatalogScan(fetch);
		} catch (error) {
			if (error instanceof CatalogScanConflictError) {
				scan = error.activeScan;
			} else {
				requestError = error instanceof Error ? error.message : "Could not start the scan.";
				return;
			}
		}
		if (scan) void follow(scan);
	}

	onMount(() => {
		if (scan && isCatalogScanActive(scan)) void follow(scan);
	});

	const counts = $derived(
		scan
			? [
					["Album directories", scan.summary.album_directories_found],
					["Candidates processed", `${scan.summary.candidates_processed} / ${scan.summary.candidates_total}`],
					["Skipped directories", scan.summary.skipped_directories],
					["Filesystem errors", scan.summary.filesystem_errors],
					["Failures", scan.summary.failures],
				]
			: [],
	);
</script>

<PageHeading
	title="Import music"
	description="Scan the configured Music directory for album folders. This scan does not change the Catalog yet."
/>

<section class="status" aria-live="polite">
	<div>
		<span class="eyebrow">Current phase</span>
		<h2>{phaseLabel}</h2>
	</div>
	<div class="controls">
		{#if active}<span class="activity">Working</span>{/if}
		<button onclick={start} disabled={active}>{active ? "Scan running" : "Start scan"}</button>
	</div>
</section>

{#if requestError}
	<p class="message error">{requestError}</p>
{/if}

{#if scan?.failure_reason}
	<p class="message error">{scan.failure_reason}</p>
{/if}

{#if scan}
	<div class="summary">
		{#each counts as [label, value]}
			<div class="count">
				<span>{label}</span>
				<strong>{value}</strong>
			</div>
		{/each}
	</div>
{:else}
	<p class="empty">No Music directory scan has run since the server started.</p>
{/if}

<style>
	.status {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
		margin-bottom: 1.5rem;
		border: 1px solid #2d2c34;
		border-radius: 0.75rem;
		padding: 1.1rem 1.25rem;
		background: #18171e;
	}

	.status h2 {
		margin: 0.2rem 0 0;
		font-size: 1.25rem;
	}

	.eyebrow,
	.count span {
		color: #aaa8b5;
		font-size: 0.85rem;
	}

	.activity {
		border-radius: 999px;
		padding: 0.3rem 0.65rem;
		color: #cbc9ff;
		background: #302d67;
		font-size: 0.85rem;
	}

	.controls {
		display: flex;
		align-items: center;
		gap: 0.75rem;
	}

	.summary {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
		gap: 0.75rem;
	}

	.count {
		display: grid;
		gap: 0.35rem;
		border: 1px solid #2d2c34;
		border-radius: 0.65rem;
		padding: 1rem;
		background: #18171e;
	}

	.count strong {
		font-size: 1.4rem;
	}

	.message,
	.empty {
		border-radius: 0.65rem;
		padding: 0.9rem 1rem;
	}

	.error {
		border: 1px solid #693c3c;
		color: #ffc2c2;
		background: #351f24;
	}

	.empty {
		border: 1px dashed #3d3b46;
		color: #aaa8b5;
	}
</style>
