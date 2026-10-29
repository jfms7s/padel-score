import type { NavKey } from './nav';

/** One 24px stroke path per place; shared by the tab bar and the rail. */
export const ICONS: Record<NavKey, string> = {
	home: 'M4 11l8-7 8 7v9a1 1 0 0 1-1 1h-4v-6H9v6H5a1 1 0 0 1-1-1z',
	organise: 'M7 4h10v4a5 5 0 0 1-10 0zM5 5H3v2a3 3 0 0 0 4 3M19 5h2v2a3 3 0 0 1-4 3M12 13v4M8 20h8',
	new: 'M12 5v14M5 12h14',
	results: 'M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z',
	people:
		'M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM2.5 20a6.5 6.5 0 0 1 13 0M16 4.5a3.5 3.5 0 0 1 0 6.5M18 14a6.5 6.5 0 0 1 3.5 6'
};
