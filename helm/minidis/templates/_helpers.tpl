{{- define "minidisservice.name" -}}
{{- default "minidis" .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* Helm required labels */}}
{{- define "minidisservice.labels" -}}
heritage: {{ .Release.Service }}
release: {{ .Release.Name }}
chart: {{ .Chart.Name }}
app: "{{ template "minidisservice.name" . }}"
{{- end -}}

{{/* matchLabels */}}
{{- define "minidisservice.matchLabels" -}}
release: {{ .Release.Name }}
app: "{{ template "minidisservice.name" . }}"
{{- end -}}