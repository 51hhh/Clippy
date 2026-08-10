export type PinPayload = {
  label: string;
  kind: "image" | "text";
  text: string | null;
  imageBase64: string | null;
  contentWidth: number;
  contentHeight: number;
  scale: number;
  opacity: number;
  locked: boolean;
  canSave: boolean;
  canEdit: boolean;
};

export type PinUpdate = {
  scale?: number;
  opacity?: number;
  locked?: boolean;
};
