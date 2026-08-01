import { createBrowserRouter } from "react-router-dom";

import { Shell } from "@/app/shell";
import { MailPlaceholder } from "@/components/mail-placeholder";
import { RequireAuth } from "@/components/require-auth";
import { LoginPage } from "@/routes/login";
import { ComposePage } from "@/features/compose/compose-page";
import { ComposeEntrance } from "@/features/compose/compose-entrance";
import { CalendarPage } from "@/features/calendar/calendar-page";
import { ContactsPage } from "@/features/contacts/contacts-page";
import { ContactDetail } from "@/features/contacts/contact-detail";
import { ContactEdit } from "@/features/contacts/contact-edit";
import { InboxRedirect } from "@/features/mail/inbox-redirect";
import { MailboxPage } from "@/features/mail/mailbox-page";
import { ConversationView } from "@/features/mail/conversation-view";
import { SearchPage } from "@/features/search/search-page";
import { SettingsPage } from "@/features/settings/settings-page";

export const router = createBrowserRouter(
  [
    { path: "/login", element: <LoginPage /> },
    {
      element: <RequireAuth />,
      children: [
        {
          path: "/",
          element: <Shell />,
          children: [
            { index: true, element: <InboxRedirect /> },
            { path: "calendar", element: <CalendarPage /> },
            {
              path: "contacts",
              element: <ContactsPage />,
              children: [
                { path: "new", element: <ContactEdit /> },
                { path: ":contactId", element: <ContactDetail /> },
                { path: ":contactId/edit", element: <ContactEdit /> },
              ],
            },
            { path: "search", element: <SearchPage /> },
            {
              path: "compose",
              element: (
                <ComposeEntrance>
                  <ComposePage />
                </ComposeEntrance>
              ),
            },
            { path: "settings", element: <SettingsPage /> },
            {
              path: ":mailboxId",
              element: <MailboxPage />,
              children: [
                {
                  index: true,
                  element: (
                    <MailPlaceholder
                      title="Select a message"
                      description="Choose a message to read it here."
                    />
                  ),
                },
                { path: ":emailId", element: <ConversationView /> },
              ],
            },
          ],
        },
      ],
    },
  ],
  { basename: "/webmail" },
);
