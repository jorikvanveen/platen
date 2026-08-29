<script lang="ts">
	import { onMount } from "svelte";
	import EmptyState from "$lib/components/EmptyState.svelte";
	import PageHeading from "$lib/components/PageHeading.svelte";
	import { API_URL } from "$lib/constants";
	import type { DownloadJob } from "$lib/dto/DownloadJob";
	import type { PageProps } from "./$types";

	let { data }: PageProps = $props();
	// svelte-ignore state_referenced_locally
	let jobs = $state<DownloadJob[]>(data.jobs);

	async function refresh() {
		const response = await fetch(`${API_URL}/downloads`);
		if (response.ok) {
			jobs = (await response.json()) as DownloadJob[];
		}
	}

	onMount(() => {
		const interval = setInterval(() => void refresh(), 2000);
		return () => clearInterval(interval);
	});
</script>

<PageHeading title="Downloads" description="Albums currently waiting to download or being placed in your Music directory." />

{#if jobs.length === 0}
	<EmptyState message="No active downloads." />
{:else}
	<div class="table-wrap">
		<table>
			<thead>
				<tr>
					<th scope="col">Album</th>
					<th scope="col">Status</th>
				</tr>
			</thead>
			<tbody>
				{#each jobs as job (job.id)}
					<tr>
						<td>{job.release_name}</td>
						<td><span class="status">{job.status}</span></td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}

<style>
	.table-wrap {
		overflow-x: auto;
	}

	table {
		width: 100%;
		border-collapse: collapse;
		border: 1px solid #302f38;
		background: #19181e;
	}

	th,
	td {
		padding: 0.85rem 1rem;
		text-align: left;
	}

	th {
		color: #aaa7b6;
		font-size: 0.85rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}

	tbody tr + tr {
		border-top: 1px solid #302f38;
	}

	.status {
		color: #c9c6ff;
		text-transform: capitalize;
	}
</style>
