<script lang="ts">
	import type { Snippet } from "svelte";
		import AlbumMetadata from "./AlbumMetadata.svelte";

	let {
		title,
		headingLevel = "h2",
		metadata,
		action,
		albumMetadata,
				actionWidth = "5rem",
	}: {
		title: string;
		headingLevel?: "h2" | "h3";
		metadata: Snippet;
		action: Snippet;
		albumMetadata?: { explicit: boolean | null; available_quality: string | null };
				actionWidth?: string;
	} = $props();
</script>

<article class:with-metadata={albumMetadata !== undefined} style:--action-width={actionWidth}>
	<div>
		<svelte:element this={headingLevel}>{title}</svelte:element>
		<div class="metadata">{@render metadata()}</div>
	</div>
	{#if albumMetadata}
		<AlbumMetadata album={albumMetadata} />
	{/if}
	{@render action()}
</article>

<style>
	article {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		align-items: center;
		gap: 1rem;
		border: 1px solid #302f38;
		border-radius: 0.8rem;
		padding: 0.85rem 1rem;
		background: #19181e;
	}

	article.with-metadata {
		grid-template-columns: minmax(0, 1fr) 21rem var(--action-width);
	}

	h2,
	h3 {
		margin: 0 0 0.22rem;
		font-size: 1rem;
	}

	.metadata {
		display: flex;
		flex-wrap: wrap;
		gap: 0.35rem 0.8rem;
		color: #9e9ba8;
		font-size: 0.9rem;
	}

	@media (max-width: 800px) {
		article.with-metadata {
			grid-template-columns: minmax(0, 1fr);
		}

	}

	@media (max-width: 620px) {
		article {
			grid-template-columns: 1fr;
		}

		article :global(button) {
			width: 100%;
		}
	}
</style>
