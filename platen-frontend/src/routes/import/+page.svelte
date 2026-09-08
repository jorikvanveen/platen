<script lang="ts">
	import { onMount } from "svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import { isCatalogScanActive } from "$lib/catalogScan";
	import { createImportController } from "$lib/importController";
	import type { CatalogScan } from "$lib/dto/CatalogScan";
	import type { PageProps } from "./$types";

	let { data }: PageProps = $props();
	// svelte-ignore state_referenced_locally -- The controller owns updates after the initial route load.
	const controller = createImportController(fetch, data.scan);
	const scan = $derived($controller.scan);

	const active = $derived(isCatalogScanActive(scan));
	const phaseLabel = $derived(getPhaseLabel(scan?.phase));

	function getPhaseLabel(phase: CatalogScan["phase"] | undefined): string {
		switch (phase) {
			case "scanning":
				return "Scanning the filesystem";
			case "matching":
				return "Matching candidates against Tidal";
			case "completed":
				return "Completed";
			case "failed":
				return "Failed";
			default:
				return "Not run yet";
		}
	}

	onMount(() => {
		void controller.resume();
		return () => controller.dispose();
	});

	const counts = $derived(
		scan
			? [
					["Album directories", scan.summary.album_directories_found],
					["Candidates processed", `${scan.summary.candidates_processed} / ${scan.summary.candidates_total}`],
					["Albums imported", scan.summary.albums_imported],
					["Locations attached", scan.summary.locations_attached],
					["Locations changed", scan.summary.locations_changed],
					["Locations unchanged", scan.summary.unchanged_locations],
					["Locations cleared", scan.summary.locations_cleared],
					["Unmatched candidates", scan.summary.unmatched_candidates],
					["Ambiguous matches", scan.summary.ambiguous_matches],
					["Duplicate locations skipped", scan.summary.duplicate_locations],
					["Skipped directories", scan.summary.skipped_directories],
					["Filesystem errors", scan.summary.filesystem_errors],
					["Tidal or database failures", scan.summary.failures],
				]
			: [],
	);
</script>

<PageHeading
	title="Import music"
	description="Scan the configured Music directory to update existing Catalog locations and import unique, high-confidence Tidal matches. Missing or inaccessible audio clears its stored location. Files and existing album metadata stay untouched."
/>

<section class="status" aria-live="polite">
	<div>
		<span class="eyebrow">Current phase</span>
		<h2>{phaseLabel}</h2>
	</div>
	<div class="controls">
		{#if active}<span class="activity">Working</span>{/if}
		<button onclick={controller.start} disabled={active || $controller.starting}>
			{active ? "Scan running" : $controller.starting ? "Starting scan" : "Start scan"}
		</button>
	</div>
</section>

{#if $controller.error}
	<p class="message error" role="alert">
		{$controller.error}
		{#if active && !$controller.following}
			<button onclick={controller.resume}>Retry progress</button>
		{/if}
	</p>
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
