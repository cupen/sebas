import { createRoot } from 'react-dom/client'
import 'antd/dist/reset.css'
import '../styles/tokens.css'
import '../styles/app.css'
import { App } from './App.js'
import { ThemeProvider } from './theme/ThemeProvider.js'

const container = document.getElementById('react-root')
if (container === null) throw new Error('missing #react-root')

// No StrictMode: its development-only double effect invocation makes every
// mount-time fetch fire twice, which the product contract (one catalog /
// usage / snapshot request per view) and the browser journeys observe as a
// real duplicate. The Lit workbench this entry replaces had no such replay.
createRoot(container).render(
  <ThemeProvider>
    <App />
  </ThemeProvider>,
)
