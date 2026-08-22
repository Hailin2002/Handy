import React from "react";
import { useTranslation } from "react-i18next";

interface ProductPlaceholderProps {
  titleKey: string;
  bodyKey: string;
}

/**
 * Minimal empty state for product views whose full implementation ships with
 * a later leaf (Meetings §6.5, Projects §6.3). Intentionally bare: no fake
 * controls, no dead actions.
 */
export const ProductPlaceholder: React.FC<ProductPlaceholderProps> = ({
  titleKey,
  bodyKey,
}) => {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col items-center justify-center text-center gap-2 py-16 max-w-md">
      <h2 className="text-lg font-semibold">{t(titleKey)}</h2>
      <p className="text-sm text-mid-gray">{t(bodyKey)}</p>
    </div>
  );
};
