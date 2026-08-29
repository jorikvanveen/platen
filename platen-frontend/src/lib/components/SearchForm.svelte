<script lang="ts">
	let {
		query = $bindable(),
		ariaLabel,
		placeholder,
		onsearch,
		loading = false
	}: {
		query: string;
		ariaLabel: string;
		placeholder: string;
		loading?: boolean;
		onsearch: () => void;
	} = $props();

	const loadingFrames = ['...', '..', '.'];
	let loadingFrame = $state(0);

	$effect(() => {
		if (!loading) {
			loadingFrame = 0;
			return;
		}

		const interval = setInterval(() => {
			loadingFrame = (loadingFrame + 1) % loadingFrames.length;
		}, 300);

		return () => clearInterval(interval);
	});
</script>

<form
	onsubmit={(event) => {
		event.preventDefault();
		onsearch();
	}}
>
	<input aria-label={ariaLabel} {placeholder} bind:value={query} />
	<button aria-label={loading ? 'Searching' : 'Search'} disabled={loading} type="submit">
		<span class="button-width" aria-hidden="true">Search</span>
		<span aria-hidden="true">{loading ? loadingFrames[loadingFrame] : 'Search'}</span>
	</button>
</form>

<style>
	form {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		margin-bottom: 2rem;
	}

	button {
		display: inline-grid;
	}

	button > span {
		grid-area: 1 / 1;
	}

	.button-width {
		visibility: hidden;
	}

	input {
		width: min(34rem, 100%);
		border: 1px solid #474652;
		border-radius: 0.65rem;
		padding: 0.72rem 0.85rem;
		color: inherit;
		background: #1b1a21;
	}

	input:focus {
		border-color: #817cff;
		outline: 2px solid #817cff33;
	}
	
	@media (max-width: 620px) {
		form {
			align-items: stretch;
			flex-direction: column;
		}
	}
</style>
