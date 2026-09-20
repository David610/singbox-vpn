import { AuthMessage } from "@/components/AuthMessage";
import { resetPassword } from "@/app/auth/actions";

type Props = {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
};

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value;
}

export default async function ResetPasswordPage({ searchParams }: Props) {
  const params = await searchParams;

  return (
    <section className="form-wrap">
      <p className="eyebrow">Account recovery</p>
      <h1 style={{ fontSize: "2.4rem" }}>Choose a new password</h1>
      <AuthMessage error={first(params.error)} message={first(params.message)} />

      <form className="form" action={resetPassword}>
        <label>
          New password
          <input
            name="password"
            type="password"
            autoComplete="new-password"
            minLength={10}
            required
          />
        </label>
        <button type="submit">Update password</button>
      </form>
    </section>
  );
}
