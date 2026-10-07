export class ArtistMonitoringState {
	monitored = $state(false);
	desired = $state<boolean | null>(null);
	pending = $state(false);
	error = $state('');

	constructor(monitored: boolean) {
		this.monitored = monitored;
	}
}
