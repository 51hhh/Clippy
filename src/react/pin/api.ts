import {
  closePin,
  copyPin,
  editPin,
  getPinPayload,
  pinReady,
  savePin,
  updatePin,
} from "../../js/api.js";
import type { PinPayload, PinUpdate } from "./types";

export const pinApi = {
  get: (label: string) => getPinPayload(label) as Promise<PinPayload>,
  ready: (label: string) => pinReady(label) as Promise<void>,
  update: (label: string, update: PinUpdate) =>
    updatePin(label, update) as Promise<PinPayload>,
  copy: (label: string) => copyPin(label) as Promise<void>,
  save: (label: string) => savePin(label) as Promise<string>,
  edit: (label: string) => editPin(label) as Promise<void>,
  close: (label: string) => closePin(label) as Promise<void>,
};
