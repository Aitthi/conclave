// Shared navigation bar imported by BOTH screens — satisfies A2_shared and carries the
// <Link> targets that make every screen reachable from config.start (A3). Uses token
// classes only, so it contributes no raw hex to the A1b budget.
import { Link } from "react-router-dom";

export default function TopNav() {
  return (
    <nav className="flex gap-4 border-b border-muted bg-surface px-6 py-3 font-sans text-sm text-ink">
      <Link to="/home" className="text-accent">
        Home
      </Link>
      <Link to="/detail" className="text-muted">
        Detail
      </Link>
    </nav>
  );
}
