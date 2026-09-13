import { Link, Outlet, useNavigate } from "@tanstack/react-router";

import { logout, useSession } from "../auth/session.ts";
import { leaveVoice } from "../voice/session.ts";
import { useGatewaySession } from "../ws/useGateway.ts";
import { Avatar } from "./Avatar.tsx";
import { Toasts } from "./Toasts.tsx";

export function AppShell() {
  const user = useSession((state) => state.user);
  const navigate = useNavigate();
  useGatewaySession(user !== null);

  const onLogout = () => {
    // Store flips first, so the header and guards react before the request
    // even leaves; the navigation is client-side.
    leaveVoice();
    void logout();
    void navigate({ to: "/login" });
  };

  return (
    <div className="min-h-dvh bg-neutral-50 text-neutral-900">
      <header className="h-14 border-b border-neutral-200 bg-white">
        <div className="flex h-full items-center justify-between px-4">
          <Link to="/" className="text-lg font-semibold tracking-tight">
            Gelabber
          </Link>
          {user ? (
            <nav className="flex items-center gap-3 text-sm">
              <Link
                to="/profile"
                className="flex items-center gap-2 rounded-full py-1 pr-3 pl-1 hover:bg-neutral-100"
                activeProps={{ className: "bg-neutral-100" }}
              >
                <Avatar name={user.name} url={user.avatar_url} />
                <span className="max-w-40 truncate font-medium">
                  {user.name}
                </span>
              </Link>
              <button
                type="button"
                onClick={onLogout}
                className="rounded-lg px-3 py-1.5 text-neutral-600 hover:bg-neutral-100 hover:text-neutral-900"
              >
                Abmelden
              </button>
            </nav>
          ) : null}
        </div>
      </header>
      <Outlet />
      <Toasts />
    </div>
  );
}
