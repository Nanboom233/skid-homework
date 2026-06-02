import ChatPage from "@/components/chat/page";
import RequireInit from "@/components/guards/RequireInit";

export default function ChatRoute() {
  return (
    <RequireInit>
      <ChatPage />
    </RequireInit>
  );
}
