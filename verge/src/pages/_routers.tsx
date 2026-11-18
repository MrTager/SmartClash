import { createBrowserRouter, Navigate, RouteObject } from 'react-router'

import Layout from './_layout'
import { navItems } from './_navigation'
import LogsPage from './logs'
import ProxyPage from './proxies'

export const router = createBrowserRouter([
  {
    path: '/',
    Component: Layout,
    children: [...navItems.map(
      (item) =>
        ({
          path: item.path,
          Component: item.Component,
        }) as RouteObject,
    ),
      { path: '/smart', element: <Navigate to="/proxies" replace /> },
      { path: '/normal', element: <Navigate to="/proxies" replace /> },
      { path: '/logs', Component: LogsPage },
      { path: '/advanced-proxies', Component: ProxyPage },
    ],
  },
])
