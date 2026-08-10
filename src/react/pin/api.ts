import {
  closePin,
  copyPin,
  editPin,
  getPinPayload,
  pinReady,
  savePin,
  updatePin,
} from "../../js/api.ts";
import type { PinPayload, PinUpdate } from "./types";

export const pinApi = {
  get: (label: string): Promise<PinPayload> => getPinPayload(label),
  ready: (label: string): Promise<void> => pinReady(label),
  update: (label: string, update: PinUpdate): Promise<PinPayload> => updatePin(label, update),
  copy: (label: string): Promise<void> => copyPin(label),
  save: (label: string): Promise<string> => savePin(label),
  edit: (label: string): Promise<void> => editPin(label),
  close: (label: string): Promise<void> => closePin(label),
};
