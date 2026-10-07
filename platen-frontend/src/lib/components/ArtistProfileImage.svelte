<script lang="ts">
	import * as Avatar from '$lib/components/ui/avatar/index.js';
	import type { Artist } from '$lib/dto/Artist';

	let { artist, compact = false }: { artist: Artist; compact?: boolean } = $props();

	const artistInitials = $derived(
		artist.name
			.trim()
			.split(/\s+/)
			.slice(0, 2)
			.map((word) => Array.from(word)[0] ?? '')
			.join('')
			.toLocaleUpperCase('en') || '?'
	);
</script>

<Avatar.Root class="size-full">
	{#if artist.profile_image_url}
		<Avatar.Image
			src={artist.profile_image_url}
			alt=""
			class="object-cover"
			loading="lazy"
			decoding="async"
		/>
	{/if}
	<Avatar.Fallback class={compact
				? 'text-[0.8125rem] font-medium text-muted-foreground'
				: 'text-3xl font-medium tracking-tight text-muted-foreground'}>
		{artistInitials}
	</Avatar.Fallback>
</Avatar.Root>
