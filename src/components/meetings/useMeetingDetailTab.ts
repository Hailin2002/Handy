// Local tab state for the meeting detail view.
//
// A custom hook is a value export, which disables react-refresh for whichever
// module holds it — and hooks conventionally live in their own file anyway.

import { useEffect, useRef, useState } from "react";
import type { MeetingDetailTab } from "./meetingDetailTabModel";

/** Local state hook so the detail view stays declarative. */
export const useMeetingDetailTab = (resetKey: string) => {
  const [tab, setTab] = useState<MeetingDetailTab>("overview");
  const lastKey = useRef(resetKey);
  useEffect(() => {
    if (lastKey.current !== resetKey) {
      lastKey.current = resetKey;
      // Opening a different meeting starts on the plain-language tab again.
      setTab("overview");
    }
  }, [resetKey]);
  return [tab, setTab] as const;
};
