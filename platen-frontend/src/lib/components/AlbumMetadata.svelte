<script lang="ts">
	let { album, stacked = false }: {
		album: { explicit: boolean | null; available_quality: string | null };
		stacked?: boolean;
	} = $props();
</script>

<dl class:stacked>
	<div><dt>Explicit</dt><dd>{album.explicit === true ? "Explicit" : album.explicit === false ? "Not explicit" : "Unknown"}</dd></div>
	<div><dt>Available quality</dt><dd>{album.available_quality ?? "Unknown"}</dd></div>
</dl>

<style>
	dl {
		display: grid;
		grid-template-columns: 8rem 12rem;
		gap: 1rem;
		margin: 0;
	}

	dt {
		color: #9e9ba8;
		font-size: 0.8rem;
		margin-bottom: 0.25rem;
	}

	dd {
		margin: 0;
		overflow-wrap: anywhere;
	}

	dl.stacked {
		grid-template-columns: minmax(0, 1fr);
		gap: 0.65rem;
	}

	@media (max-width: 800px) {
		dl {
			grid-template-columns: minmax(0, 1fr);
			gap: 0.65rem;
		}
	}
</style>
