<script lang="ts">
	import { LoaderCircle } from '@lucide/svelte';
	import { Badge } from '$lib/components/ui/badge/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import type { DownloadJob } from '$lib/dto/DownloadJob';

	let {
		title,
		jobs,
		emptyMessage,
		showFailureReason = false,
		cancellingId = null,
		oncancel
	}: {
		title: string;
		jobs: DownloadJob[];
		emptyMessage: string;
		showFailureReason?: boolean;
		cancellingId?: string | null;
		oncancel?: (job: DownloadJob, button: HTMLElement) => void;
	} = $props();

	const statusLabels = {
		queued: 'Queued',
		running: 'Downloading',
		succeeded: 'Succeeded',
		failed: 'Failed',
		cancelled: 'Cancelled'
	};
</script>

<section aria-label={title}>
	<div class="section-heading">
		<h2>{title}</h2>
		<Badge variant="secondary">{jobs.length}</Badge>
	</div>
	<Card.Root class="gap-0 py-0">
		<Card.Content class="p-0">
			{#if jobs.length === 0}
				<p class="empty-state">{emptyMessage}</p>
			{:else}
				<table>
					<caption class="sr-only">{title}</caption>
					<thead>
						<tr>
							<th scope="col">Album</th>
							<th scope="col">Artist</th>
							<th scope="col">Status</th>
							{#if showFailureReason}<th scope="col">Failure reason</th>{/if}
						</tr>
					</thead>
					<tbody>
						{#each jobs as job (job.id)}
							<tr>
								<td class="album">
									<span class="mobile-label" aria-hidden="true">Album</span>
									{job.release_name ?? job.album_id}
								</td>
								<td>
									<span class="mobile-label" aria-hidden="true">Artist</span>
									{job.artists.map((artist) => artist.name).join(', ') || 'Unknown'}
								</td>
								<td>
									<span class="mobile-label" aria-hidden="true">Status</span>
									<div class="status-cell">
										<Badge
											variant={job.status === 'failed' ? 'outline' : job.status === 'running' ? 'default' : 'secondary'}
											class={job.status === 'failed' ? 'text-destructive' : undefined}
										>
											{#if job.status === 'running'}
												<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
											{/if}
											{statusLabels[job.status]}
										</Badge>
										{#if job.status === 'queued' && oncancel}
											<Button
												variant="outline"
												size="sm"
												disabled={cancellingId !== null}
												onclick={(event) => oncancel?.(job, event.currentTarget)}
											>
												{cancellingId === job.id ? 'Cancelling...' : 'Cancel'}
												<span class="sr-only"> {job.release_name ?? job.album_id}</span>
											</Button>
										{/if}
									</div>
								</td>
								{#if showFailureReason}
									<td class="failure">
										{#if job.failure_reason}
											<span class="mobile-label" aria-hidden="true">Failure reason</span>
											{job.failure_reason}
										{/if}
									</td>
								{/if}
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
		</Card.Content>
	</Card.Root>
</section>

<style>
	.section-heading {
		display: flex;
		align-items: center;
		gap: 0.625rem;
		margin-bottom: 1rem;
	}

	h2 {
		font-size: 1.0625rem;
		font-weight: 600;
		letter-spacing: -0.02em;
	}

	.empty-state {
		padding: 2.5rem 1.5rem;
		color: var(--muted-foreground);
		text-align: center;
	}

	table {
		width: 100%;
		table-layout: fixed;
		border-collapse: collapse;
		font-size: 0.8125rem;
	}

	th,
	td {
		padding: 1rem;
		text-align: left;
		vertical-align: top;
		overflow-wrap: anywhere;
	}

	th {
		color: var(--foreground);
		font-size: 0.75rem;
		font-weight: 500;
		background: var(--muted);
	}

	th:first-child {
		width: 28%;
	}

	tbody tr + tr {
		border-top: 1px solid var(--border);
	}

	.album {
		font-weight: 500;
	}

	.status-cell {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 0.625rem;
	}

	.failure {
		color: var(--destructive);
	}

	.mobile-label {
		display: none;
	}

	@media (max-width: 48rem) {
		thead {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip-path: inset(50%);
			white-space: nowrap;
		}

		table,
		tbody {
			display: block;
		}

		tr {
			display: grid;
			grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
			gap: 1rem;
			padding: 1rem;
		}

		td {
			padding: 0;
		}

		.album,
		.failure {
			grid-column: 1 / -1;
		}

		.failure:empty {
			display: none;
		}

		.mobile-label {
			display: block;
			margin-bottom: 0.375rem;
			font-size: 0.75rem;
			font-weight: 400;
			color: var(--muted-foreground);
		}
	}
</style>
