import React, { useEffect, useState } from "react";
import { Textarea } from "@/components/ui/Textarea";

interface AssistantPromptFieldProps {
  value: string;
  disabled?: boolean;
  placeholder?: string;
  ariaLabel: string;
  onCommit: (value: string) => void;
}

/**
 * System-prompt editor for one assistant profile.
 *
 * Commits on blur rather than per keystroke: the prompt is a paragraph, and a
 * write per character would mean one keychain-adjacent settings save per
 * character typed.
 */
export const AssistantPromptField: React.FC<AssistantPromptFieldProps> = ({
  value,
  disabled = false,
  placeholder,
  ariaLabel,
  onCommit,
}) => {
  const [draft, setDraft] = useState(value);

  // The store wins on refresh; a prompt reset elsewhere must show up here.
  useEffect(() => {
    setDraft(value);
  }, [value]);

  return (
    <Textarea
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={() => onCommit(draft)}
      placeholder={placeholder}
      aria-label={ariaLabel}
      disabled={disabled}
      rows={6}
      className="w-full"
    />
  );
};
