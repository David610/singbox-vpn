import Link from "next/link";
import { AuthMessage } from "@/components/AuthMessage";
import { signup } from "@/app/auth/actions";

type Props = {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
};

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

export default async function SignupPage({ searchParams }: Props) {
  const params = await searchParams;

  return (
    <section className="form-wrap">
      <p className="eyebrow">Account</p>
      <h1 style={{ fontSize: "2.4rem" }}>Create account</h1>
      <AuthMessage error={first(params.error)} message={first(params.message)} />

      <form className="form" action={signup}>
        <label>
          Email
          <input name="email" type="email" autoComplete="email" required />
        </label>
        <label>
          Password
          <input
            name="password"
            type="password"
            autoComplete="new-password"
            minLength={10}
            required
          />
        </label>
        <label className="checkbox">
          <input name="legal" type="checkbox" value="yes" required />
          <span>
            I agree to the <Link href="/terms">Terms</Link> and have read the{" "}
            <Link href="/privacy">Privacy Policy</Link>.
          </span>
        </label>
        <button type="submit">Create account</button>
      </form>

      <p style={{ marginTop: "1rem", color: "#666", fontSize: "0.9rem" }}>
        Already registered? <Link href="/auth/login">Log in</Link>
      </p>
    </section>
  );
}
