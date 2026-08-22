import { Mic2 } from "lucide-react";

const PRODUCT_NAME = "Speak";

interface SpeakTextLogoProps {
  width?: number | string;
  className?: string;
}

const SpeakTextLogo: React.FC<SpeakTextLogoProps> = ({
  width = 120,
  className = "",
}) => (
  <div
    className={`flex items-center justify-center gap-2 text-logo-primary ${className}`}
    style={{ width }}
  >
    <span className="flex h-8 w-8 items-center justify-center rounded-xl bg-logo-primary/15">
      <Mic2 className="h-5 w-5" strokeWidth={2.25} />
    </span>
    <span className="text-2xl font-bold tracking-tight text-text">
      {PRODUCT_NAME}
    </span>
  </div>
);

export default SpeakTextLogo;
