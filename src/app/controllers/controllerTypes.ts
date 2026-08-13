import type { Dispatch, SetStateAction } from "react";
import { translations } from "../../domain/i18n";

export type AppText = (typeof translations)[keyof typeof translations];
export type SetBusy = Dispatch<SetStateAction<boolean>>;
export type SetNotice = (message: string) => void;
