<script lang="ts">
	import * as Avatar from '$lib/components/ui/avatar/index.js';
	import type { Artist } from '$lib/dto/Artist';

	let { artist }: { artist: Artist } = $props();

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
	<Avatar.Fallback class="text-3xl font-medium tracking-tight text-muted-foreground">
		{artistInitials}
	</Avatar.Fallback>
</Avatar.Root>
