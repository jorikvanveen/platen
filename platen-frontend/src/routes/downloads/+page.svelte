<script lang="ts">
	import { onMount } from "svelte";
	import DownloadJobTable from "$lib/components/DownloadJobTable.svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import { API_URL } from "$lib/constants";
	import type { Downloads } from "$lib/dto/Downloads";
	import type { PageProps } from "./$types";

	let { data }: PageProps = $props();
	// svelte-ignore state_referenced_locally
	let downloads = $state<Downloads>(data.downloads);

	async function refresh() {
		const response = await fetch(`${API_URL}/downloads`);
		if (response.ok) {
			downloads = (await response.json()) as Downloads;
		}
	}

	onMount(() => {
		const interval = setInterval(() => void refresh(), 2000);
		return () => clearInterval(interval);
	});
</script>

<PageHeading title="Downloads" description="Active downloads and the latest 100 completed attempts." />

<div class="download-tables">
	<DownloadJobTable
		title="Active downloads"
		jobs={downloads.active}
		emptyMessage="No active downloads."
	/>

	<DownloadJobTable
		title="History"
		jobs={downloads.history}
		emptyMessage="No download history."
		showFailureReason
	/>
</div>

<style>
	.download-tables {
		display: grid;
		gap: 2.5rem;
	}
</style>
