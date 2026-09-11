import { API_URL } from '$lib/constants';
import type { DownloadJob } from '$lib/dto/DownloadJob';

type DownloadRequestResult =
	| { outcome: 'accepted'; job: DownloadJob }
	| { outcome: 'downloaded' }
	| { outcome: 'failed'; message: string; status?: number };

export async function queueAlbumDownload(albumId: string): Promise<DownloadRequestResult> {
	try {
		const response = await fetch(`${API_URL}/albums/${encodeURIComponent(albumId)}/download`, {
			method: 'POST'
		});
		if (response.status === 409) return { outcome: 'downloaded' };
		if (!response.ok) {
			return {
				outcome: 'failed',
				status: response.status,
				message:
					response.status === 429
						? 'The download queue is full. Try again later.'
						: response.status === 503
							? 'Downloads are unavailable. Try again later.'
							: 'Could not queue this download. Try again.'
			};
		}
		return { outcome: 'accepted', job: (await response.json()) as DownloadJob };
	} catch {
		return {
			outcome: 'failed',
			message: 'Could not reach the download service. Try again.'
		};
	}
}
