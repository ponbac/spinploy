import { type ClassValue, clsx } from "clsx";
import dayjs from "dayjs";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
	return twMerge(clsx(inputs));
}

export function formatDate(date: Date | string): string {
	return dayjs(date).format("YYYY-MM-DD");
}

export function formatTime(date: Date | string): string {
	return dayjs(date).format("HH:mm");
}

export function formatDateTime(date: Date | string): string {
	return dayjs(date).format("YYYY-MM-DD HH:mm");
}
