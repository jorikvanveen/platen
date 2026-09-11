<script lang="ts">
	import { Check, CircleAlert, LoaderCircle } from '@lucide/svelte';
	import type { CatalogScan } from '$lib/dto/CatalogScan';

	let { scan, stale = false }: { scan: CatalogScan; stale?: boolean } = $props();
	const summary = $derived(scan.summary);
	const count = new Intl.NumberFormat();
	const phaseLabels: Record<CatalogScan['phase'], string> = {
		scanning: 'Reading the Music directory',
		matching: 'Matching & importing albums',
		completed: 'Scan complete',
		failed: 'Scan failed'
	};
	const stages = $derived([
		{
			phase: 'scanning',
			title: 'Read Music directory',
			complete: scan.phase === 'matching' || scan.phase === 'completed',
			waiting: false,
			stats: [
				{ label: 'Album directories found', value: summary.album_directories_found },
				{ label: 'Filesystem errors', value: summary.filesystem_errors }
			]
		},
		{
			phase: 'matching',
			title: 'Match & import albums',
			complete: scan.phase === 'completed',
			waiting: scan.phase === 'scanning',
			stats: [
				{ label: 'Albums imported', value: summary.albums_imported },
				{ label: 'No Tidal match', value: summary.unmatched_candidates }
			]
		}
	]);
	const locations = $derived([
		{ label: 'Attached', value: summary.locations_attached },
		{ label: 'Changed', value: summary.locations_changed },
		{ label: 'Unchanged', value: summary.unchanged_locations },
		{ label: 'Cleared', value: summary.locations_cleared }
	]);
	const skipped = $derived([
		{ label: 'Directories skipped', value: summary.skipped_directories },
		{ label: 'Ambiguous matches', value: summary.ambiguous_matches },
		{ label: 'Duplicate locations', value: summary.duplicate_locations },
		{ label: 'Processing failures', value: summary.failures }
	]);
	const hasSkipped = $derived(skipped.some((stat) => stat.value > 0));
</script>

<div class="scan-report">
	<div class="report-header">
		<h2 aria-live="polite" aria-atomic="true">
			{#if scan.phase === 'completed'}
				<Check size={20} aria-hidden="true" />
			{:else if scan.phase === 'failed'}
				<CircleAlert size={20} aria-hidden="true" />
			{:else if !stale}
				<LoaderCircle size={20} class="motion-safe:animate-spin" aria-hidden="true" />
			{/if}
			{stale ? 'Last update' : phaseLabels[scan.phase]}
		</h2>
		{#if summary.candidates_total > 0 || scan.phase === 'completed'}
			<p class="candidate-count">
				<strong>{count.format(summary.candidates_processed)}</strong>
				<span> / {count.format(summary.candidates_total)} candidates processed</span>
			</p>
		{/if}
	</div>

	{#if scan.phase === 'failed'}
		<div class="failure" role="alert">
			<p>{scan.failure_reason ?? 'The scan stopped unexpectedly.'}</p>
			<p>Changes made before the failure are kept.</p>
		</div>
	{/if}

	<ol class="pipeline" aria-label="Scan stages" role="list">
		{#each stages as stage, index (stage.phase)}
			{@const working = scan.phase === stage.phase}
			<li class:working={working && !stale} aria-current={working && !stale ? 'step' : undefined}>
				<div class="stage-heading">
					<span class="stage-marker" aria-hidden="true">
						{#if stage.complete}
							<Check size={16} />
						{:else if working && !stale}
							<LoaderCircle size={16} class="motion-safe:animate-spin" />
						{:else}
							{index + 1}
						{/if}
					</span>
					<div>
						<h3>{stage.title}</h3>
						{#if stage.complete}
							<p>Done</p>
						{:else if working}
							<p>{stale ? 'Last seen working' : 'Working'}</p>
						{:else if stage.waiting}
							<p>Waiting</p>
						{/if}
					</div>
				</div>
				<dl class="stage-stats">
					{#each stage.stats as stat (stat.label)}
						<div>
							<dt>{stat.label}</dt>
							<dd>
								{#if stage.waiting}
									<span aria-hidden="true">–</span>
									<span class="sr-only">Not started</span>
								{:else}
									{count.format(stat.value)}
								{/if}
							</dd>
						</div>
					{/each}
				</dl>
			</li>
		{/each}
	</ol>

	<section class="locations" aria-labelledby="locations-heading">
		<div class="section-heading">
			<h3 id="locations-heading">Catalog locations</h3>
			<p>Across the scan</p>
		</div>
		<div>
			<dl class="location-stats">
				{#each locations as stat (stat.label)}
					<div>
						<dt>{stat.label}</dt>
						<dd>{count.format(stat.value)}</dd>
					</div>
				{/each}
			</dl>
			{#if summary.locations_cleared > 0}
				<p class="note">
					Cleared locations had missing or inaccessible audio. Their albums stay in the catalog.
				</p>
			{/if}
		</div>
	</section>

	{#if hasSkipped}
		<details class="skipped">
			<summary>
				<span>Skipped & failed</span>
				{#if summary.skipped_directories > 0}
					<span class="skipped-count">{count.format(summary.skipped_directories)} skipped</span>
				{/if}
				{#if summary.failures > 0}
					<span class="skipped-count">
						{count.format(summary.failures)} {summary.failures === 1 ? 'failure' : 'failures'}
					</span>
				{/if}
			</summary>
			<dl class="skipped-stats">
				{#each skipped as stat (stat.label)}
					<div>
						<dt>{stat.label}</dt>
						<dd>{count.format(stat.value)}</dd>
					</div>
				{/each}
			</dl>
			<p class="note">
				Counts can overlap. Skipped directories include unmatched albums and duplicate locations.
			</p>
		</details>
	{/if}
</div>

<style>
	.scan-report {
		overflow: hidden;
		border: 1px solid var(--border);
		border-radius: var(--radius-xl);
		background: var(--card);
	}

	.report-header {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
		padding: 1.5rem;
		border-bottom: 1px solid var(--border);
	}

	h2 {
		display: flex;
		align-items: center;
		gap: 0.625rem;
		font-size: 1rem;
		font-weight: 600;
	}

	.candidate-count {
		font-size: 0.875rem;
		font-variant-numeric: tabular-nums;
	}

	.candidate-count strong {
		font-weight: 600;
	}

	.candidate-count span {
		color: var(--muted-foreground);
	}

	.failure {
		display: grid;
		gap: 0.375rem;
		padding: 1.25rem 1.5rem;
		border-bottom: 1px solid var(--border);
		color: var(--destructive);
		font-size: 0.875rem;
		overflow-wrap: anywhere;
	}

	.pipeline li,
	.locations {
		display: grid;
		grid-template-columns: minmax(14rem, 1fr) minmax(0, 2fr);
		align-items: start;
		gap: 2rem;
		padding: 1.75rem 1.5rem;
	}

	.pipeline li + li,
	.locations,
	.skipped {
		border-top: 1px solid var(--border);
	}

	.pipeline li {
		border-left: 3px solid transparent;
		padding-left: calc(1.5rem - 3px);
	}

	.pipeline li.working {
		border-left-color: var(--foreground);
	}

	.stage-heading {
		display: flex;
		align-items: flex-start;
		gap: 0.875rem;
	}

	.stage-marker {
		display: flex;
		width: 1.75rem;
		height: 1.75rem;
		flex-shrink: 0;
		align-items: center;
		justify-content: center;
		border: 1px solid var(--border);
		border-radius: 50%;
		font-size: 0.75rem;
		font-weight: 600;
	}

	h3 {
		font-size: 0.875rem;
		font-weight: 600;
		line-height: 1.75rem;
	}

	.stage-heading p,
	.section-heading p {
		color: var(--muted-foreground);
		font-size: 0.75rem;
	}

	dl {
		display: grid;
		gap: 1.25rem;
		min-width: 0;
	}

	.stage-stats {
		grid-template-columns: repeat(2, minmax(0, 1fr));
	}

	.location-stats,
	.skipped-stats {
		grid-template-columns: repeat(4, minmax(0, 1fr));
	}

	dt {
		color: var(--muted-foreground);
		font-size: 0.8125rem;
		line-height: 1.5;
	}

	dd {
		margin-top: 0.375rem;
		font-size: 1.5rem;
		font-weight: 550;
		font-variant-numeric: tabular-nums;
		line-height: 1.2;
		overflow-wrap: anywhere;
	}

	.note {
		margin-top: 1rem;
		color: var(--muted-foreground);
		font-size: 0.8125rem;
		line-height: 1.6;
	}

	.skipped {
		padding: 1.25rem 1.5rem;
	}

	summary {
		cursor: pointer;
		border-radius: 0.25rem;
		font-size: 0.875rem;
		font-weight: 600;
	}

	summary:focus-visible {
		outline: 2px solid var(--ring);
		outline-offset: 0.25rem;
	}

	.skipped-count {
		margin-left: 0.75rem;
		color: var(--muted-foreground);
		font-weight: 400;
	}

	.skipped-stats {
		margin-top: 1.5rem;
	}

	@media (max-width: 56rem) {
		.pipeline li,
		.locations {
			grid-template-columns: 1fr;
			gap: 1.25rem;
		}
	}

	@media (max-width: 32rem) {
		.report-header,
		.failure,
		.pipeline li,
		.locations,
		.skipped {
			padding: 1.25rem;
		}

		.pipeline li {
			padding-left: calc(1.25rem - 3px);
		}

		.location-stats,
		.skipped-stats {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
	}
</style>
