<script lang="ts">
	import { LoaderCircle, Trash2 } from '@lucide/svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { API_URL } from '$lib/constants';
	import type { Artist } from '$lib/dto/Artist';

	let { artist }: { artist: Artist } = $props();
	let open = $state(false);
	let removing = $state(false);
	let removed = $state(false);
	let removalError = $state('');
	let stale = $state(false);
	let cancelButton = $state<HTMLButtonElement | null>(null);

	async function removeArtist() {
		if (removing || removed || stale) return;
		removing = true;
		removalError = '';

		try {
			const response = await fetch(`${API_URL}/artists/${encodeURIComponent(artist.id)}`, {
				method: 'DELETE'
			});
			if (response.status === 409) {
				stale = true;
				removalError = 'This artist now has albums in the catalog and cannot be removed. Refresh the page to see them.';
				return;
			}
			if (response.status === 404) {
				stale = true;
				removalError = 'This artist could not be found. Refresh the page to see the current catalog.';
				return;
			}
			if (response.status !== 204) {
				removalError = 'Could not remove this artist. Try again.';
				return;
			}
			removed = true;
		} catch {
			removalError = 'Could not confirm removal. Refresh the page before trying again.';
			stale = true;
			return;
		} finally {
			removing = false;
		}

		try {
			await goto(resolve('/'), { invalidateAll: true });
		} catch {
			removalError = 'Artist removed, but the artist list could not be loaded.';
		}
	}
</script>

<AlertDialog.Root bind:open>
	<AlertDialog.Trigger>
		{#snippet child({ props })}
			<Button {...props} variant="outline">
				<Trash2 aria-hidden="true" />
				Remove Artist
			</Button>
		{/snippet}
	</AlertDialog.Trigger>
	<AlertDialog.Content
		class="max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] overflow-y-auto"
		onOpenAutoFocus={(event) => {
			event.preventDefault();
			cancelButton?.focus();
		}}
		onEscapeKeydown={(event) => {
			if (removing) event.preventDefault();
		}}
		onCloseAutoFocus={(event) => {
			if (removed) event.preventDefault();
		}}
	>
		<AlertDialog.Header>
			<AlertDialog.Title class="break-words">Remove {artist.name}?</AlertDialog.Title>
		</AlertDialog.Header>
		{#if removalError}<p class="removal-error" role="alert">{removalError}</p>{/if}
		<AlertDialog.Footer>
			<AlertDialog.Cancel bind:ref={cancelButton} disabled={removing}>Cancel</AlertDialog.Cancel>
			{#if removed}
				<Button href={resolve('/')} data-sveltekit-reload>Go to artists</Button>
			{:else if stale}
				<Button onclick={() => window.location.reload()}>Refresh page</Button>
			{:else}
				<Button
					variant="destructive"
					class="bg-destructive text-white hover:bg-destructive dark:bg-destructive dark:text-black dark:hover:bg-destructive"
					disabled={removing}
					onclick={removeArtist}
				>
					{#if removing}<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />{/if}
					{removing ? 'Removing...' : 'Remove Artist'}
				</Button>
			{/if}
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<style>
	.removal-error {
		color: var(--destructive);
		font-size: 0.8125rem;
		line-height: 1.5;
		overflow-wrap: anywhere;
	}
</style>
