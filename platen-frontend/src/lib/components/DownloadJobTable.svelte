<script lang="ts">
	import EmptyState from "$lib/components/EmptyState.svelte";
	import type { DownloadJob } from "$lib/dto/DownloadJob";

	let {
		title,
		jobs,
		emptyMessage,
		showFailureReason = false,
	}: {
		title: string;
		jobs: DownloadJob[];
		emptyMessage: string;
		showFailureReason?: boolean;
	} = $props();
</script>

<section>
	<h2>{title}</h2>

	{#if jobs.length === 0}
		<EmptyState message={emptyMessage} />
	{:else}
		<div class="table-wrap">
			<table>
				<thead>
					<tr>
						<th scope="col">Album</th>
						<th scope="col">Status</th>
						{#if showFailureReason}
							<th scope="col">Failure reason</th>
						{/if}
					</tr>
				</thead>
				<tbody>
					{#each jobs as job (job.id)}
						<tr>
							<td>{job.release_name}</td>
							<td><span class="status">{job.status}</span></td>
							{#if showFailureReason}
								<td class="failure">{job.failure_reason ?? ""}</td>
							{/if}
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</section>

<style>
	h2 {
	margin: 0 0 1rem;
	font-size: 1.2rem;
}

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

.failure {
	color: #d9a6a6;
}
</style>
