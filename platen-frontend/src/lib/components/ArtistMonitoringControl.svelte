<script lang="ts">
	import { Eye, EyeClosed } from '@lucide/svelte';
	import { ArtistMonitoringState } from '$lib/artist-monitoring.svelte';
	import { Button } from '$lib/components/ui/button/index.js';
	import { API_URL } from '$lib/constants';
	import type { Artist } from '$lib/dto/Artist';
	import type { ArtistMonitoringUpdate } from '$lib/dto/ArtistMonitoringUpdate';

	let {
		artist,
		state = new ArtistMonitoringState(artist.monitored),
		showError = true
	}: {
		artist: Artist;
		state?: ArtistMonitoringState;
		showError?: boolean;
	} = $props();

	const checked = $derived(state.desired ?? state.monitored);
	const errorId = $derived('monitoring-error-' + artist.id);

	async function updateMonitoring(desired: boolean) {
		const requestState = state;
		if (requestState.pending || desired === requestState.monitored) return;
		const previous = requestState.monitored;
		const artistId = artist.id;
		const artistName = artist.name;
		requestState.desired = desired;
		requestState.pending = true;
		requestState.error = '';

		try {
			const update: ArtistMonitoringUpdate = { monitored: desired };
			const response = await fetch(`${API_URL}/artists/${encodeURIComponent(artistId)}`, {
				method: 'PATCH',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify(update)
			});
			if (!response.ok) throw new Error('Artist monitoring update failed');
			const savedArtist = (await response.json()) as Artist;
			requestState.monitored = savedArtist.monitored;
		} catch {
			requestState.monitored = previous;
			requestState.error = `Could not update monitoring for ${artistName}. Try again.`;
		} finally {
			requestState.desired = null;
			requestState.pending = false;
		}
	}
</script>

<Button
	variant="ghost"
	size="icon"
	class="size-11 rounded-md"
	role="checkbox"
	aria-label={'Monitor ' + artist.name}
	aria-checked={checked}
	aria-busy={state.pending}
	aria-describedby={state.error ? errorId : undefined}
	title={checked ? 'Stop monitoring ' + artist.name : 'Monitor ' + artist.name}
	disabled={state.pending}
	onclick={() => updateMonitoring(!checked)}
>
	{#if checked}
		<Eye class="size-5 text-foreground" aria-hidden="true" strokeWidth={1.75} />
	{:else}
		<EyeClosed class="size-5 text-muted-foreground" aria-hidden="true" strokeWidth={1.75} />
	{/if}
</Button>

{#if showError && state.error}
	<p class="monitoring-error" id={errorId} role="alert">{state.error}</p>
{/if}

<style>
	.monitoring-error {
		margin-top: 0.5rem;
		color: var(--destructive);
		font-size: 0.875rem;
	}
</style>
