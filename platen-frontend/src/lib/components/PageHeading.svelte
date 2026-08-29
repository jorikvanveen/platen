<script lang="ts">
	type Action = {
		href: string;
		label: string;
	};

	let { title, description, actions = [] }: { title: string; description: string; actions?: Action[] } = $props();
</script>

<div class="page-heading">
	<div>
		<h1>{title}</h1>
		<p>{description}</p>
	</div>
	{#if actions.length > 0}
		<div class="actions">
			{#each actions as action (action.href)}
				<a href={action.href}>{action.label}</a>
			{/each}
		</div>
	{/if}
</div>

<style>
	.page-heading {
		display: flex;
		align-items: end;
		justify-content: space-between;
		gap: 1rem;
		margin-bottom: 2rem;
	}

	p {
		max-width: 42rem;
		margin-bottom: 0;
		color: #aaa7b6;
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.7rem;
	}

	a {
		border: 1px solid #474652;
		border-radius: 0.55rem;
		padding: 0.55rem 0.8rem;
		text-decoration: none;
	}

	@media (max-width: 620px) {
		.page-heading {
			align-items: stretch;
			flex-direction: column;
		}
	}
</style>
