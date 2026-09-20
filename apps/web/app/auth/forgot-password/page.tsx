import Link from "next/link";
import { AuthMessage } from "@/components/AuthMessage";
import { requestPasswordReset } from "@/app/auth/actions";

type Props = {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
};

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

export default async function ForgotPasswordPage({ searchParams }: Props) {
  const params = await searchParams;

  return (
    <section className="form-wrap">
      <p className="eyebrow">Account recovery</p>
      <h1 style={{ fontSize: "2.4rem" }}>Reset password</h1>
      <p className="lead" style={{ fontSize: "0.95rem", marginTop: "0.8rem" }}>
        We will send a reset link if the address belongs to an account.
      </p>
      <AuthMessage error={first(params.error)} message={first(params.message)} />

      <form className="form" action={requestPasswordReset}>
        <label>
          Email
          <input name="email" type="email" autoComplete="email" required />
        </label>
        <button type="submit">Send reset link</button>
      </form>

      <p style={{ marginTop: "1rem", fontSize: "0.9rem" }}>
        <Link href="/auth/login">Back to login</Link>
      </p>
    </section>
  );
}
