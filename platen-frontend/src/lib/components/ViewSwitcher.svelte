<script lang="ts" module>
	export type CatalogView = 'cards' | 'list';
</script>

<script lang="ts">
	import { LayoutGrid, List } from '@lucide/svelte';
	import { Button } from '$lib/components/ui/button/index.js';

	let {
		value,
		label = 'Display view',
		onchange
	}: {
		value: CatalogView;
		label?: string;
		onchange: (value: CatalogView) => void;
	} = $props();
</script>

<div class="view-switcher" role="group" aria-label={label}>
	<Button
		variant={value === 'cards' ? 'secondary' : 'ghost'}
		size="icon"
		class="size-11 rounded-md"
		aria-label="Card view"
		aria-pressed={value === 'cards'}
		title="Card view"
		onclick={() => onchange('cards')}
	>
		<LayoutGrid class="size-[18px]" aria-hidden="true" strokeWidth={1.75} />
	</Button>
	<Button
		variant={value === 'list' ? 'secondary' : 'ghost'}
		size="icon"
		class="size-11 rounded-md"
		aria-label="List view"
		aria-pressed={value === 'list'}
		title="List view"
		onclick={() => onchange('list')}
	>
		<List class="size-[18px]" aria-hidden="true" strokeWidth={1.75} />
	</Button>
</div>

<style>
	.view-switcher {
		display: inline-flex;
		gap: 0.125rem;
		padding: 0.25rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-xl);
		background: var(--card);
	}
</style>
