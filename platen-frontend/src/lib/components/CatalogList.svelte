<script lang="ts" generics="Item">
	import type { Snippet } from 'svelte';

	let {
		items,
		key,
		label,
		row,
		header
	}: {
		items: Item[];
		key: (item: Item) => string | number;
		label: string;
		row: Snippet<[Item]>;
		header?: Snippet;
	} = $props();
</script>

<div class="catalog-list-container">
	{@render header?.()}
	<ul class="catalog-list" aria-label={label} role="list">
		{#each items as item (key(item))}
			<li>{@render row(item)}</li>
		{/each}
	</ul>
</div>

<style>
	.catalog-list-container {
		overflow: hidden;
		border: 1px solid var(--border);
		border-radius: var(--radius-xl);
		background: var(--card);
	}

	.catalog-list {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	li + li {
		border-top: 1px solid var(--border);
	}
</style>
