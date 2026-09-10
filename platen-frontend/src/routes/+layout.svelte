<script lang="ts">
	let { children } = $props();
</script>

<svelte:head>
	<title>Platen</title>
</svelte:head>

<main>
	{@render children()}
</main>
