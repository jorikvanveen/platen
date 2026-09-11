<script lang="ts">
	import { LoaderCircle, Trash2 } from '@lucide/svelte';
	import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
	import { Button } from '$lib/components/ui/button/index.js';
	import { Checkbox } from '$lib/components/ui/checkbox/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import { API_URL } from '$lib/constants';
	import type { Album } from '$lib/dto/Album';
	import type { AlbumDeletionPreview } from '$lib/dto/AlbumDeletionPreview';
	import type { AlbumDeletionRequest } from '$lib/dto/AlbumDeletionRequest';
	import type { AlbumDeletionResult } from '$lib/dto/AlbumDeletionResult';

	let {
		album,
		disabled = false,
		downloadActive = false,
		ondelete
	}: {
		album: Album;
		disabled?: boolean;
		downloadActive?: boolean;
		ondelete: (result: AlbumDeletionResult) => void;
	} = $props();

	const componentId = $props.id();
	const checkboxId = `${componentId}-delete-files`;
	const downloaded = $derived(album.relative_path !== null);
	let open = $state(false);
	let deleteFiles = $state(false);
	let deleting = $state(false);
	let deleted = $state(false);
	let deletionError = $state('');
	let preview = $state<AlbumDeletionPreview | null>(null);
	let previewError = $state('');
	let cancelButton = $state<HTMLButtonElement | null>(null);

	$effect(() => {
		if (!open) return;
		deleteFiles = false;
		deletionError = '';
		preview = null;
		previewError = '';
		if (!downloaded) return;

		const previewUrl = `${API_URL}/albums/${encodeURIComponent(album.id)}/deletion-preview`;
		const controller = new AbortController();

		async function loadPreview() {
			try {
				const response = await fetch(previewUrl, { signal: controller.signal });
				if (!response.ok) throw new Error('Deletion preview failed');
				const result = (await response.json()) as AlbumDeletionPreview;
				if (!controller.signal.aborted) preview = result;
			} catch {
				if (!controller.signal.aborted) {
					previewError = 'Could not check the album location. You can still delete it from the catalog.';
				}
			}
		}

		void loadPreview();
		return () => controller.abort();
	});

	async function deleteAlbum() {
		if (deleting || deleted || (deleteFiles && !preview?.absolute_path)) return;
		deleting = true;
		deletionError = '';
		let result: AlbumDeletionResult;

		try {
			const request: AlbumDeletionRequest = { delete_files: deleteFiles };
			const response = await fetch(`${API_URL}/albums/${encodeURIComponent(album.id)}`, {
				method: 'DELETE',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify(request)
			});
			if (!response.ok) {
				// File removal can succeed before catalog deletion fails, so preserve the backend's explanation.
				deletionError = (await response.text()) || 'Could not delete this album. Try again.';
				return;
			}
			result = (await response.json()) as AlbumDeletionResult;
		} catch {
			deletionError = 'Could not confirm deletion. Check the catalog before trying again.';
			return;
		} finally {
			deleting = false;
		}

		deleted = true;
		open = false;
		ondelete(result);
	}
</script>

<AlertDialog.Root bind:open>
	<AlertDialog.Trigger {disabled}>
		{#snippet child({ props })}
			<Button
				{...props}
				variant="outline"
				size="icon"
				class="size-10 shrink-0"
				aria-label={`Delete ${album.title}`}
				title="Delete album"
			>
				<Trash2 aria-hidden="true" />
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
			if (deleting) event.preventDefault();
		}}
		onCloseAutoFocus={(event) => {
			if (deleted) event.preventDefault();
		}}
	>
		<AlertDialog.Header>
			<AlertDialog.Title class="break-words">Delete {album.title}?</AlertDialog.Title>
			<AlertDialog.Description>
				This removes the album from the catalog for all credited artists.
				{#if downloadActive}
					Its queued or running download will not be cancelled.
				{/if}
			</AlertDialog.Description>
		</AlertDialog.Header>

		{#if downloaded && preview?.absolute_path !== null}
			<div class="file-option">
				<div class="checkbox-row">
					<Checkbox
						id={checkboxId}
						bind:checked={deleteFiles}
						disabled={deleting || !preview?.absolute_path}
						aria-describedby={`${checkboxId}-description`}
					/>
					<Label for={checkboxId} class="leading-relaxed">Also delete files from disk</Label>
				</div>
				<div id={`${checkboxId}-description`} class="file-description" aria-live="polite">
					{#if previewError}
						<p>{previewError}</p>
					{:else if !preview}
						<p>Checking album location...</p>
					{:else if deleteFiles}
						<p>This directory and everything in it will be permanently deleted.</p>
						<code>{preview.absolute_path}</code>
					{/if}
				</div>
			</div>
		{/if}

		{#if deletionError}<p class="deletion-error" role="alert">{deletionError}</p>{/if}
		<AlertDialog.Footer>
			<AlertDialog.Cancel bind:ref={cancelButton} disabled={deleting}>Cancel</AlertDialog.Cancel>
			<Button
				variant="destructive"
				class="bg-destructive text-white hover:bg-destructive dark:bg-destructive dark:text-black dark:hover:bg-destructive"
				disabled={deleting}
				onclick={deleteAlbum}
			>
				{#if deleting}<LoaderCircle class="motion-safe:animate-spin" aria-hidden="true" />{/if}
				{deleting ? 'Deleting...' : deleteFiles ? 'Delete album and files' : 'Delete album'}
			</Button>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<style>
	.checkbox-row {
		display: flex;
		align-items: center;
		gap: 0.75rem;
	}

	.file-description {
		color: var(--muted-foreground);
		font-size: 0.8125rem;
		line-height: 1.5;
	}

	.file-description p {
		margin-top: 0.75rem;
	}

	.file-description code {
		display: block;
		margin-top: 0.5rem;
		color: var(--foreground);
		overflow-wrap: anywhere;
	}

	.deletion-error {
		color: var(--destructive);
		font-size: 0.8125rem;
		line-height: 1.5;
		overflow-wrap: anywhere;
	}
</style>
