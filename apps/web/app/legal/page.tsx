import { productName } from "@/lib/env";

export const dynamic = "force-dynamic";

function field(name: string): string {
  return process.env[name]?.trim() || `[configure ${name} before launch]`;
}

export default function LegalPage() {
  const vat = process.env.LEGAL_VAT_ID?.trim();

  return (
    <article className="prose">
      <h1>Legal Notice</h1>
      <p><strong>{field("LEGAL_NAME")}</strong></p>
      <p style={{ whiteSpace: "pre-line" }}>{field("LEGAL_ADDRESS")}</p>
      <p>Email: {field("LEGAL_EMAIL")}</p>
      {vat && <p>VAT ID: {vat}</p>}

      <h2>Service</h2>
      <p>{productName()} provides subscription-based VPN access.</p>

      <div className="notice error">
        This page intentionally fails visibly when required operator details are
        missing. Do not launch publicly until the legal identity and address have
        been filled and reviewed.
      </div>
    </article>
  );
}
