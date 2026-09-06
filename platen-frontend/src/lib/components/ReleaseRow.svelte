<script lang="ts">
	import type { Snippet } from "svelte";

	let {
		title,
		headingLevel = "h2",
		metadata,
		action,
		discovery,
	}: {
		title: string;
		headingLevel?: "h2" | "h3";
		metadata: Snippet;
		action: Snippet;
		discovery?: { explicit: boolean | null; available_quality: string | null };
	} = $props();
</script>

<article class:discovery={discovery !== undefined}>
	<div>
		<svelte:element this={headingLevel}>{title}</svelte:element>
		<div class="metadata">{@render metadata()}</div>
	</div>
	{#if discovery}
		<dl>
			<div><dt>Explicit</dt><dd>{discovery.explicit === true ? "Explicit" : discovery.explicit === false ? "Not explicit" : "Unknown"}</dd></div>
			<div><dt>Available quality</dt><dd>{discovery.available_quality ?? "Unknown"}</dd></div>
		</dl>
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

	article.discovery {
		grid-template-columns: minmax(0, 1fr) 21rem 5rem;
	}

	dl {
		display: grid;
		grid-template-columns: 8rem 12rem;
		gap: 1rem;
		margin: 0;
	}

	dt {
		color: #9e9ba8;
		font-size: 0.8rem;
		margin-bottom: 0.25rem;
	}

	dd {
		margin: 0;
		overflow-wrap: anywhere;
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
		article.discovery {
			grid-template-columns: minmax(0, 1fr);
		}

		dl {
			grid-template-columns: minmax(0, 1fr);
			gap: 0.65rem;
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
