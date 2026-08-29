<script lang="ts">
	let { message }: { message: string } = $props();
</script>

<div>{message}</div>

<style>
	div {
		border: 1px dashed #3a3942;
		border-radius: 0.8rem;
		padding: 2rem;
		color: #aaa7b6;
		text-align: center;
	}
</style>
