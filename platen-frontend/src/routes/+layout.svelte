<script lang="ts">
	import '../app.css';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { Button } from '$lib/components/ui/button/index.js';

	let { children } = $props();
</script>

<svelte:head>
	<title>Platen</title>
</svelte:head>

<a class="skip-link" href="#main-content">Skip to content</a>

<header class="site-header">
	<div class="header-content">
		<a class="brand" href={resolve('/')} aria-label="Platen home">
			Platen
		</a>
		<nav aria-label="Main">
			<Button
				href={resolve('/')}
				variant={page.route.id === '/' ? 'secondary' : 'ghost'}
				aria-current={page.route.id === '/' ? 'page' : undefined}
			>
				Artists
			</Button>
			<Button
				href={resolve('/search')}
				variant={page.route.id === '/search' ? 'secondary' : 'ghost'}
				aria-current={page.route.id === '/search' ? 'page' : undefined}
			>
				Search albums
			</Button>
		</nav>
	</div>
</header>

<main id="main-content" tabindex="-1">
	{@render children()}
</main>

<style>
	.site-header {
		border-bottom: 1px solid var(--border);
		background: var(--card);
	}

	.header-content,
	main {
		width: min(100% - 3rem, 80rem);
		margin-inline: auto;
	}

	.header-content {
		display: flex;
		min-height: 5rem;
		align-items: center;
		justify-content: space-between;
		gap: 1.5rem;
	}

	.brand {
		border-radius: var(--radius);
		color: var(--foreground);
		font-size: 1.25rem;
		font-weight: 650;
		letter-spacing: -0.04em;
		text-decoration: none;
	}

	nav {
		display: flex;
		align-items: center;
		gap: 0.25rem;
	}

	main {
		padding-block: 3.5rem 5rem;
	}

	.skip-link {
		position: fixed;
		top: 0.75rem;
		left: 0.75rem;
		z-index: 10;
		transform: translateY(calc(-100% - 1rem));
		border-radius: var(--radius);
		padding: 0.75rem 1rem;
		background: var(--primary);
		color: var(--primary-foreground);
	}

	.skip-link:focus {
		transform: translateY(0);
	}

	.brand:focus-visible,
	.skip-link:focus-visible {
		outline: 2px solid var(--ring);
		outline-offset: 4px;
	}

	@media (max-width: 40rem) {
		.header-content,
		main {
			width: calc(100% - 2rem);
		}

		.header-content {
			min-height: 4.5rem;
		}

		main {
			padding-block: 2rem 3rem;
		}
	}
</style>
