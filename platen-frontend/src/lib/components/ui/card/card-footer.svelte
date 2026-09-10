<script lang="ts">
	import { mergeClasses, type WithElementRef } from "$lib/utils.js";
	import type { HTMLAttributes } from "svelte/elements";

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: WithElementRef<HTMLAttributes<HTMLDivElement>> = $props();
</script>

<div
	bind:this={ref}
	data-slot="card-footer"
	class={mergeClasses("bg-muted/50 rounded-b-xl border-t p-(--card-spacing) flex items-center", className)}
	{...restProps}
>
	{@render children?.()}
</div>
