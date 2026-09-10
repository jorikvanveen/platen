<script lang="ts">
	import { Avatar as AvatarPrimitive } from "bits-ui";
	import { mergeClasses } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		...restProps
	}: AvatarPrimitive.ImageProps = $props();
</script>

<AvatarPrimitive.Image
	bind:ref
	data-slot="avatar-image"
	class={mergeClasses("rounded-full aspect-square size-full object-cover", className)}
	{...restProps}
/>
