<script lang="ts">
	import { Eye, EyeClosed } from '@lucide/svelte';
	import { API_URL } from '$lib/constants';
	import type { Artist } from '$lib/dto/Artist';
	import type { ArtistMonitoringUpdate } from '$lib/dto/ArtistMonitoringUpdate';

	let { artist }: { artist: Artist } = $props();

	let monitored = $derived(artist.monitored);
	let pending = $state<boolean | null>(null);
	let checked = $derived(pending ?? monitored);

	async function updateMonitoring(desired: boolean) {
		if (pending !== null || desired === monitored) return;
		const previous = monitored;
		pending = desired;

		try {
			const request: ArtistMonitoringUpdate = { monitored: desired };
			const response = await fetch(`${API_URL}/artists/${encodeURIComponent(artist.id)}`, {
				method: 'PATCH',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify(request)
			});
			if (!response.ok) throw new Error('Artist monitoring update failed');
			const savedArtist = (await response.json()) as Artist;
			monitored = savedArtist.monitored;
		} catch {
			monitored = previous;
		} finally {
			pending = null;
		}
	}
</script>

<button
	type="button"
	role="checkbox"
	aria-label="Monitor artist"
	aria-checked={checked}
	title={checked ? 'Stop monitoring artist' : 'Monitor artist'}
	disabled={pending !== null}
	onclick={() => updateMonitoring(!monitored)}
>
	{#if checked}
		<Eye size={26} aria-hidden="true" />
	{:else}
		<EyeClosed size={26} aria-hidden="true" />
	{/if}
</button>

<style>
	button {
		display: inline-flex;
		width: 2.5rem;
		height: 2.5rem;
		align-items: center;
		justify-content: center;
		border-radius: 0.375rem;
		color: var(--muted-foreground);
		cursor: pointer;
		transition: color 150ms, background-color 150ms;
	}

	button[aria-checked='true'] {
		color: var(--foreground);
	}

	button:hover:not(:disabled) {
		background: var(--accent);
		color: var(--foreground);
	}

	button:focus-visible {
		outline: 2px solid var(--ring);
		outline-offset: 2px;
	}

	button:disabled {
		cursor: wait;
		opacity: 0.5;
	}
</style>
