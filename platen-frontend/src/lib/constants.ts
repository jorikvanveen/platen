import { env } from '$env/dynamic/public';

export const API_URL = env.PUBLIC_PLATEN_BACKEND_URL ?? '/api';
