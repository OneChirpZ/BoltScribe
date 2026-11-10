import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { expect, it } from "vitest";
import { createBrowserPreviewState } from "../app/api/previewData";
import { translations } from "../domain/i18n";
import CorrectionPage from "./CorrectionPage";

function PromptPage() {
  const [config, setConfig] = useState(() => createBrowserPreviewState().config);
  return <CorrectionPage section="prompt" config={config} onChange={setConfig} text={translations["zh-CN"]} />;
}

it("updates the assembled prompt and preserves requirements when switching editor tabs", () => {
  render(<PromptPage />);
  fireEvent.change(screen.getByRole("textbox", {name: /纠错要求/}), {target:{value:"保留技术术语"}});
  const preview = screen.getByRole("region", {name: translations["zh-CN"].correction.preview});
  expect(preview.textContent).toContain("保留技术术语");
  expect((screen.getByRole("textbox", {name: "模板正文"}) as HTMLTextAreaElement).value).toContain("{{user_requirements}}");
  fireEvent.click(screen.getByRole("tab", {name:translations["zh-CN"].correction.systemPrompt}));
  fireEvent.click(screen.getByRole("tab", {name:"模板"}));
  expect((screen.getByRole("textbox", {name:"纠错要求"}) as HTMLTextAreaElement).value).toBe("保留技术术语");
});
