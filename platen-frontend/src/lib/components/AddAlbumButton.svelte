<script lang="ts">
	import { API_URL } from "$lib/constants";

	let { albumId }: { albumId: string } = $props();
	let state: "idle" | "adding" | "added" | "error" = $state("idle");

	async function addAlbum() {
		state = "adding";
		try {
			const response = await fetch(`${API_URL}/albums/${albumId}`, { method: "POST" });
			state = response.ok ? "added" : "error";
		} catch {
			state = "error";
		}
	}
</script>

<button
	class:success={state === "added"}
	class:error={state === "error"}
	disabled={state === "adding" || state === "added"}
	onclick={addAlbum}
>
	{#if state === "adding"}
		Adding…
	{:else if state === "added"}
		Added
	{:else if state === "error"}
		Retry
	{:else}
		Add
	{/if}
</button>
