<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { CircleAlert, LoaderCircle, RefreshCw } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import * as Card from '$lib/components/ui/card/index.js';
	import { API_URL } from '$lib/constants';
	import type { DownloadJob } from '$lib/dto/DownloadJob';
	import type { Downloads } from '$lib/dto/Downloads';
	import DownloadJobTable from './DownloadJobTable.svelte';

	let downloads = $state<Downloads | null>(null);
	let refreshing = $state(false);
	let refreshError = $state('');
	let actionMessage = $state('');
	let cancellingId = $state<string | null>(null);
	let heading: HTMLHeadingElement;
	let mounted = false;
	let timer: ReturnType<typeof setTimeout> | undefined;
	let refreshPromise: Promise<void> | null = null;
	let requestController: AbortController | null = null;

	async function requestQueue<Result>(path: string, method: 'GET' | 'DELETE' = 'GET') {
		const controller = new AbortController();
		requestController = controller;
		const timeout = setTimeout(() => controller.abort(), 15000);
		try {
			const response = await fetch(`${API_URL}${path}`, { method, signal: controller.signal });
			const result = response.ok ? ((await response.json()) as Result) : null;
			return { ok: response.ok, status: response.status, result };
		} finally {
			clearTimeout(timeout);
			if (requestController === controller) requestController = null;
		}
	}

	async function loadDownloads() {
		try {
			const response = await requestQueue<Downloads>('/downloads');
			if (!response.ok || !response.result) throw new Error('Could not load downloads');
			if (mounted) {
				downloads = response.result;
				refreshError = '';
			}
		} catch {
			if (mounted) {
				refreshError = downloads
					? 'Could not refresh downloads. Showing the last update.'
					: 'Could not load downloads.';
			}
		}
	}

	function scheduleRefresh() {
		clearTimeout(timer);
		if (mounted && cancellingId === null) {
			timer = setTimeout(() => void refresh(), 2000);
		}
	}

	async function refresh() {
		if (!mounted || cancellingId !== null) return;
		if (refreshPromise) return refreshPromise;
		clearTimeout(timer);
		refreshing = true;
		refreshPromise = loadDownloads();
		try {
			await refreshPromise;
		} finally {
			refreshPromise = null;
			refreshing = false;
			scheduleRefresh();
		}
	}

	async function cancel(job: DownloadJob, button: HTMLElement) {
		if (!mounted || cancellingId !== null || job.status !== 'queued') return;
		cancellingId = job.id;
		actionMessage = '';
		clearTimeout(timer);
		let restoreFocus = document.activeElement === button;
		const trackFocus = (event: FocusEvent) => {
			if (event.target !== button) restoreFocus = false;
		};
		document.addEventListener('focusin', trackFocus);

		try {
			// Finish the previous read before mutating so it cannot overwrite the cancellation.
			await refreshPromise;
			if (!mounted) return;
			const response = await requestQueue<DownloadJob>(
				`/downloads/${encodeURIComponent(job.id)}`,
				'DELETE'
			);
			if (!mounted) return;
			if (downloads && response.ok && response.result) {
				// A failed refresh must not restore a job whose cancellation was confirmed.
				downloads = {
					active: downloads.active.filter((existing) => existing.id !== job.id),
					history: [
						response.result,
						...downloads.history.filter((existing) => existing.id !== job.id)
					]
				};
			} else if (downloads && response.status === 404) {
				downloads = {
					active: downloads.active.filter((existing) => existing.id !== job.id),
					history: downloads.history.filter((existing) => existing.id !== job.id)
				};
			}
			const albumName = job.release_name ?? job.album_id;
			actionMessage = response.ok
				? `Cancelled ${albumName}.`
				: response.status === 409
					? `${albumName} is no longer queued and cannot be cancelled.`
					: response.status === 404
						? 'This download is no longer in the queue.'
						: 'Could not cancel this download. Try again.';
			await loadDownloads();
		} catch {
			if (mounted) {
				actionMessage = 'Could not cancel this download. Try again.';
				await loadDownloads();
			}
		} finally {
			cancellingId = null;
			await tick();
			document.removeEventListener('focusin', trackFocus);
			if (mounted && restoreFocus && !button.isConnected) heading.focus();
			scheduleRefresh();
		}
	}

	onMount(() => {
		mounted = true;
		void refresh();
		return () => {
			mounted = false;
			clearTimeout(timer);
			requestController?.abort();
		};
	});
</script>

<svelte:head>
	<title>Downloads · Platen</title>
	<meta name="description" content="View your download queue and recent download history." />
</svelte:head>

<section aria-labelledby="downloads-heading">
	<div class="page-header">
		<h1 id="downloads-heading" bind:this={heading} tabindex="-1">Downloads</h1>
		<Button variant="outline" disabled={refreshing || cancellingId !== null} onclick={() => void refresh()}>
			<RefreshCw class={refreshing ? 'motion-safe:animate-spin' : ''} aria-hidden="true" />
			Refresh
		</Button>
	</div>

	{#if refreshError}
		<div class="error-banner" role="alert">
			<CircleAlert size={18} aria-hidden="true" />
			<p>{refreshError}</p>
			<Button variant="outline" size="sm" disabled={refreshing || cancellingId !== null} onclick={() => void refresh()}>
				Try again
			</Button>
		</div>
	{/if}
	<p class="action-message" role="status">{actionMessage}</p>

	{#if downloads}
		<div class="download-sections">
			<DownloadJobTable
				title="Active downloads"
				jobs={downloads.active}
				emptyMessage="No active downloads."
				{cancellingId}
				oncancel={(job, button) => void cancel(job, button)}
			/>
			<DownloadJobTable
				title="History"
				jobs={downloads.history}
				emptyMessage="No download history."
				showFailureReason
			/>
		</div>
	{:else if !refreshError}
		<Card.Root class="gap-0 py-0">
			<Card.Content class="p-0">
				<div class="loading-state" role="status">
					<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
					Loading downloads...
				</div>
			</Card.Content>
		</Card.Root>
	{/if}
</section>

<style>
	.page-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 1rem;
		margin-bottom: 2rem;
	}

	h1 {
		font-size: clamp(1.875rem, 4vw, 2.25rem);
		font-weight: 650;
		letter-spacing: -0.04em;
		line-height: 1.2;
	}

	h1:focus-visible {
		outline: 2px solid var(--ring);
		outline-offset: 0.25rem;
		border-radius: 0.25rem;
	}

	.download-sections {
		display: grid;
		gap: 2rem;
	}

	.error-banner {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 0.75rem;
		margin-bottom: 1rem;
		padding: 1rem;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--destructive);
		font-size: 0.875rem;
	}

	.error-banner p {
		flex: 1;
		min-width: 12rem;
	}

	.action-message {
		font-size: 0.875rem;
		overflow-wrap: anywhere;
	}

	.action-message:not(:empty) {
		margin-bottom: 1.5rem;
	}

	.loading-state {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 0.75rem;
		min-height: 12rem;
		color: var(--muted-foreground);
		font-size: 0.875rem;
	}
</style>
