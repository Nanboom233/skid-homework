import RequireInit from "@/components/guards/RequireInit";
import ScanPage from "@/components/pages/ScanPage";

export default function HomePage() {
  return (
    <RequireInit>
      <ScanPage />
    </RequireInit>
  );
}
