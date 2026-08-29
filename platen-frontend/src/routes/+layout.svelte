<script lang="ts">
	import favicon from '$lib/assets/favicon.svg';
	import '../app.css';

	let { children } = $props();
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>Platen</title>
</svelte:head>

<header class="site-header">
	<div class="shell">
		<a class="brand" href="/">Platen</a>
	</div>
</header>

<main class="shell">
	{@render children()}
</main>

<style>
	.shell {
		width: min(72rem, calc(100% - 2rem));
		margin: 0 auto;
	}

	.site-header {
		border-bottom: 1px solid #2d2c34;
		background: #15141a;
	}

	.site-header .shell {
		display: flex;
		align-items: center;
		justify-content: space-between;
		min-height: 4rem;
		gap: 1rem;
	}

	.brand {
		font-weight: 750;
		text-decoration: none;
		letter-spacing: -0.02em;
	}


	main.shell {
		padding-block: clamp(2rem, 6vw, 4rem);
	}

</style>
