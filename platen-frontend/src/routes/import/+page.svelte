<script lang="ts">
	import { onMount } from 'svelte';
	import { CircleAlert, FolderSearch, LoaderCircle, RefreshCw } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { API_URL } from '$lib/constants';
	import type { CatalogScan } from '$lib/dto/CatalogScan';
	import ScanReport from './ScanReport.svelte';

	let scan = $state<CatalogScan | null>(null);
	let loaded = $state(false);
	let refreshing = $state(false);
	let starting = $state(false);
	let refreshError = $state('');
	let startError = $state('');
	let actionMessage = $state('');
	const active = $derived(scan?.phase === 'scanning' || scan?.phase === 'matching');

	let mounted = false;
	let timer: ReturnType<typeof setTimeout> | undefined;
	let refreshPromise: Promise<void> | null = null;
	let requestController: AbortController | null = null;

	async function requestScan(method: 'GET' | 'POST' = 'GET') {
		const controller = new AbortController();
		requestController = controller;
		const timeout = setTimeout(() => controller.abort(), 15000);
		try {
			const response = await fetch(`${API_URL}/catalog/scan`, {
				method,
				signal: controller.signal
			});
			const accepted =
				method === 'GET' ? response.ok : response.status === 202 || response.status === 409;
			if (!accepted) throw new Error('Scan request failed');
			const result = (await response.json()) as CatalogScan | null;
			if (method === 'POST' && !result) throw new Error('Missing scan status');
			return { scan: result, alreadyRunning: response.status === 409 };
		} finally {
			clearTimeout(timeout);
			if (requestController === controller) requestController = null;
		}
	}

	async function loadScan() {
		try {
			const response = await requestScan();
			if (!mounted) return;
			scan = response.scan;
			loaded = true;
			refreshError = '';
			if (active) startError = '';
			else actionMessage = '';
		} catch {
			if (mounted) {
				refreshError = loaded
					? 'Could not refresh scan status. Showing the last update.'
					: 'Could not load scan status.';
			}
		}
	}

	function scheduleRefresh() {
		clearTimeout(timer);
		if (mounted && !starting && (active || refreshError)) {
			timer = setTimeout(() => void refresh(), 2000);
		}
	}

	async function refresh() {
		if (!mounted || starting) return;
		if (refreshPromise) return refreshPromise;
		clearTimeout(timer);
		refreshing = true;
		refreshPromise = loadScan();
		try {
			await refreshPromise;
		} finally {
			refreshPromise = null;
			refreshing = false;
			scheduleRefresh();
		}
	}

	async function start() {
		if (!mounted || starting || active || !loaded || refreshError) return;
		starting = true;
		startError = '';
		actionMessage = '';
		clearTimeout(timer);
		try {
			// A previous read must not overwrite the newly started scan.
			await refreshPromise;
			if (!mounted || refreshError || active) return;
			const response = await requestScan('POST');
			if (!mounted) return;
			scan = response.scan;
			actionMessage = response.alreadyRunning
				? 'A scan is already running. Following its progress.'
				: '';
		} catch {
			if (mounted) {
				startError = 'Could not confirm that the scan started. Checking its status.';
				// The server may have accepted the scan even if its response was lost.
				await loadScan();
				if (mounted) {
					if (active) {
						startError = '';
						actionMessage = 'A scan is running. Following its progress.';
					} else {
						startError = 'Could not confirm that the scan started. Review the status before trying again.';
					}
				}
			}
		} finally {
			starting = false;
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
	<title>Scan music · Platen</title>
	<meta
		name="description"
		content="Scan your Music directory to import albums and update catalog locations."
	/>
</svelte:head>

<section aria-labelledby="scan-heading">
	<div class="page-header">
		<div>
			<h1 id="scan-heading">Scan music</h1>
			<p class="description">
				Import albums from your Music directory and update catalog locations. Files stay untouched.
			</p>
		</div>
		<div class="page-actions">
			<Button
				variant="outline"
				size="icon"
				aria-label="Refresh scan status"
				title="Refresh scan status"
				disabled={refreshing || starting}
				onclick={() => void refresh()}
			>
				<RefreshCw class={refreshing ? 'motion-safe:animate-spin' : ''} aria-hidden="true" />
			</Button>
			<Button
				disabled={!loaded || starting || active || Boolean(refreshError)}
				onclick={() => void start()}
			>
				{#if starting || active}
					<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
				{:else}
					<FolderSearch aria-hidden="true" />
				{/if}
				{starting ? 'Starting...' : active ? 'Scan running' : scan ? 'Scan again' : 'Start scan'}
			</Button>
		</div>
	</div>

	{#if refreshError}
		<div class="error-banner" role="alert">
			<CircleAlert size={18} aria-hidden="true" />
			<p>{refreshError}</p>
			<Button
				variant="outline"
				size="sm"
				disabled={refreshing || starting}
				onclick={() => void refresh()}
			>
				Try again
			</Button>
		</div>
	{/if}
	{#if startError}
		<p class="start-error" role="alert">{startError}</p>
	{/if}
	<p class="action-message" role="status">{actionMessage}</p>

	{#if scan}
		<ScanReport {scan} stale={Boolean(refreshError)} />
	{:else if loaded}
		<div class="empty-state">
			<FolderSearch size={36} strokeWidth={1.5} aria-hidden="true" />
			<h2>No scans yet</h2>
			<p>Scan results are kept until the server restarts.</p>
		</div>
	{:else if !refreshError}
		<div class="loading-state" role="status">
			<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />
			Loading scan status...
		</div>
	{/if}
</section>

<style>
	.page-header {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 1.5rem;
		margin-bottom: 2rem;
	}

	h1 {
		font-size: clamp(1.875rem, 4vw, 2.25rem);
		font-weight: 650;
		letter-spacing: -0.04em;
		line-height: 1.2;
	}

	.description {
		max-width: 36rem;
		margin-top: 0.75rem;
		color: var(--muted-foreground);
		font-size: 0.875rem;
		line-height: 1.6;
	}

	.page-actions {
		display: flex;
		flex-shrink: 0;
		align-items: center;
		gap: 0.5rem;
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

	.start-error,
	.action-message {
		font-size: 0.875rem;
		overflow-wrap: anywhere;
	}

	.start-error {
		margin-bottom: 1rem;
		color: var(--destructive);
	}

	.action-message:not(:empty) {
		margin-bottom: 1rem;
	}

	.empty-state,
	.loading-state {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 0.75rem;
		min-height: 18rem;
		border-block: 1px solid var(--border);
		color: var(--muted-foreground);
		font-size: 0.875rem;
	}

	.empty-state {
		flex-direction: column;
		text-align: center;
		padding: 2rem;
	}

	.empty-state h2 {
		color: var(--foreground);
		font-size: 1.125rem;
		font-weight: 600;
	}

	@media (max-width: 48rem) {
		.page-header {
			flex-direction: column;
		}
	}
</style>
