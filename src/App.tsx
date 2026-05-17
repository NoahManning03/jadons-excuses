import { Navigate, Route, Routes } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { Dashboard } from "./pages/Dashboard";
import { Activity } from "./pages/Activity";
import { Trends } from "./pages/Trends";
import { Insights } from "./pages/Insights";
import { AdvancedSettings } from "./pages/AdvancedSettings";
import { Settings } from "./pages/Settings";

function App() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route path="/" element={<Navigate to="/dashboard" replace />} />
        <Route path="/dashboard" element={<Dashboard />} />
        <Route path="/activity" element={<Activity />} />
        <Route path="/trends" element={<Trends />} />
        <Route path="/insights" element={<Insights />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="/settings/advanced" element={<AdvancedSettings />} />
      </Route>
    </Routes>
  );
}

export default App;
