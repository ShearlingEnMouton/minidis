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

{{/* Image tag: image.tag if set, else appVersion, with a -lite suffix for the lite variant */}}
{{- define "minidisservice.imageTag" -}}
  {{- if not (has .Values.variant (list "full" "lite")) -}}
    {{- fail (printf "variant must be full or lite, got %v" .Values.variant) -}}
  {{- end -}}
  {{- $suffix := ternary "-lite" "" (eq .Values.variant "lite") -}}
{{- .Values.image.tag | default (printf "%s%s" .Chart.AppVersion $suffix) -}}
{{- end -}}
